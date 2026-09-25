/**
 * Network glue between the game loop and Pylon.
 *
 * Poses and combat go through the `island` shard (shards/island) with
 * `@pylonsync/realtime`:
 *   - the local player's moves go up at 20 Hz as the distance moved since
 *     the last one. The shard applies them within a speed budget; when it
 *     clamps one, prediction (the shard's position plus the moves it has
 *     not processed yet) puts the player back where the shard has them.
 *   - other players are drawn 100 ms behind the shard, between the frames
 *     around that time, from the interpolated entities.
 *   - hits and respawns are shard inputs; health lives in the shard.
 *
 * Destroyed blocks still go to the database (destroyBlocks), batched.
 */
import { callFn } from "@pylonsync/react";
import {
  connectShardGame,
  type InterpolatedEntity,
  type Predictor,
  type ShardGame,
} from "@pylonsync/realtime";
import * as THREE from "three";
import type { EventBus, FrameCtx, GameSystem } from "./engine";
import type { Player } from "./player";

/** Inputs the island shard takes (`Input` in shards/island/src/lib.rs). */
export type IslandInput =
  | "join"
  | { move: { dx: number; dy: number; dz: number; heading: number; pitch: number } }
  | { hit: { target: number; damage: number } }
  | "spawn"
  | "leave";

/** Component ids (shards/island/src/lib.rs). */
const AVATAR = 1;
const LOOK = 2;
const HEALTH = 3;

const SHARD_ID = "island-main";
/** Move inputs per second while moving. */
const SEND_HZ = 20;
/** A standing player still sends a move this often, to stay on the island. */
const HEARTBEAT_S = 1;
/** Rejoin when the shard has not shown our entity for this long. */
const REJOIN_S = 3;
/** A difference from the shard's position smaller than this is rounding. */
const CORRECT_M = 0.3;
/** A larger one is a spawn or a rejoin: teleport instead of shifting. */
const TELEPORT_M = 5;
/** How often to mark the Avatar row as in use (spawnAvatar prunes rows
 *  unused for 30 minutes). */
const TOUCH_S = 5 * 60;

interface Vec3 {
  x: number;
  y: number;
  z: number;
}

/** Another player as drawn this frame. */
export interface RemotePose {
  avatarId: string;
  entity: number;
  x: number;
  y: number;
  z: number;
  heading: number;
  pitch: number;
  health: number;
}

/** A move shifts the player. The shard picks spawn points, so a join or
 *  spawn has no effect to predict: the correction after it moves the
 *  player there. */
function step(s: Vec3, input: IslandInput): Vec3 {
  if (typeof input === "object" && "move" in input) {
    return { x: s.x + input.move.dx, y: s.y + input.move.dy, z: s.z + input.move.dz };
  }
  return s;
}

function readLook(bytes: Uint8Array | undefined): { heading: number; pitch: number } {
  if (!bytes || bytes.length < 4) return { heading: 0, pitch: 0 };
  const view = new DataView(bytes.buffer, bytes.byteOffset, 4);
  return { heading: view.getInt16(0, true) / 10_000, pitch: view.getInt16(2, true) / 10_000 };
}

function lerpAngle(a: number, b: number, t: number): number {
  let d = b - a;
  while (d > Math.PI) d -= Math.PI * 2;
  while (d < -Math.PI) d += Math.PI * 2;
  return a + d * t;
}

function percentile(values: number[], p: number): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const idx = Math.min(sorted.length - 1, Math.floor(sorted.length * p));
  return sorted[idx];
}

export class Net implements GameSystem {
  readonly name = "net";

  /** Other players this frame, interpolated. */
  readonly remotes: RemotePose[] = [];
  /** Our health as the shard has it, or null before we are in. */
  selfHealth: number | null = null;

  private shard: ShardGame<IslandInput> | null = null;
  private me: Predictor<Vec3, IslandInput> | null = null;
  private avatarId: string | null = null;
  private selfEntity: number | null = null;
  /** Where the local player was when it sent its last move. */
  private readonly lastSent = new THREE.Vector3();
  private lastSentAt = -Infinity;
  private lastLook = { heading: Infinity, pitch: Infinity };
  private lastJoinAt = -Infinity;
  private lastSeenSelfAt = 0;
  private touchTimer: ReturnType<typeof setInterval> | null = null;
  /** Called when the server no longer has our Avatar row. */
  onIdentityLost: (() => void) | null = null;
  private dead = false;
  private readonly decoder = new TextDecoder();
  /** Avatar id per entity, decoded once per entity. */
  private readonly avatarOf = new Map<number, { bytes: Uint8Array; id: string }>();
  private readonly entityOf = new Map<string, number>();
  private readonly pool: RemotePose[] = [];

  private readonly latencies: number[] = [];
  private mutationsInWindow: number[] = [];
  private pendingKeys: string[] = [];
  private flushInFlight = false;

  constructor(
    private readonly player: Player,
    events: EventBus,
  ) {
    // The shooter syncs only directly-destroyed keys; collapse is
    // derived deterministically on every client (see buildings.ts).
    events.on("blocksDestroyed", ({ keys }) => {
      this.pendingKeys.push(...keys);
    });
    // Combat: the shooter reports hits; the shard range-checks them and
    // caps the damage.
    events.on("playerHit", ({ avatarId, damage }) => {
      const target = this.entityOf.get(avatarId);
      if (target === undefined) return;
      this.shard?.send({
        hit: { target, damage: Math.max(1, Math.min(255, Math.round(damage))) },
      });
    });
    // Death sequence finished locally: the shard picks the spawn point.
    events.on("respawnRequested", () => {
      this.shard?.send("spawn");
    });
  }

  /** Join the island as avatar `avatarId`. */
  connect(avatarId: string) {
    if (this.shard) return;
    this.avatarId = avatarId;
    const shard = connectShardGame<IslandInput>(SHARD_ID, {
      subscriberId: avatarId,
      // A new ticket for every connection attempt: they expire.
      ticket: async () => {
        try {
          return (await callFn<{ ticket: string }>("joinIsland", {})).ticket;
        } catch (err) {
          // The row was pruned (the tab slept for half an hour): this
          // identity cannot join again.
          if ((err as { code?: string }).code === "NOT_FOUND") this.onIdentityLost?.();
          throw err;
        }
      },
      tickRate: 20,
      // A respawn moves a player across the island: jump, do not slide.
      interpolation: { snapDistance: 12 },
    });
    this.shard = shard;
    this.me = shard.predict<Vec3>(step);
    // Wall-clock, not the frame loop: a hidden tab pauses the loop, and
    // spawnAvatar prunes rows untouched for 30 minutes.
    this.touchTimer = setInterval(() => {
      callFn("touchAvatar", {}).catch(() => {
        // The next interval retries; joinIsland touches the row too.
      });
    }, TOUCH_S * 1000);
    shard.onOpen(() => this.join());
    shard.onReplication((table, _summary, _tick, ack) => this.reconcile(ack));
  }

  /** The player is dead (no moves) or alive. */
  setDead(dead: boolean) {
    this.dead = dead;
  }

  get hudStats(): { p50: number; p95: number; mutPerSec: number; rtt: number | null } {
    const now = Date.now();
    this.mutationsInWindow = this.mutationsInWindow.filter((t) => now - t < 1000);
    const recent = this.latencies.slice(-100);
    const rtt = this.shard?.rttMs;
    return {
      p50: Math.round(percentile(recent, 0.5)),
      p95: Math.round(percentile(recent, 0.95)),
      mutPerSec: this.mutationsInWindow.length,
      rtt: rtt === null || rtt === undefined ? null : Math.round(rtt),
    };
  }

  private track<T>(promise: Promise<T>): Promise<T> {
    const t0 = performance.now();
    return promise.then((r) => {
      this.latencies.push(performance.now() - t0);
      if (this.latencies.length > 200) this.latencies.shift();
      this.mutationsInWindow.push(Date.now());
      return r;
    });
  }

  private join() {
    if (this.shard?.send("join")) {
      this.lastSent.copy(this.player.position);
      this.lastJoinAt = performance.now() / 1000;
    }
  }

  /** Our entity id, found by its AVATAR component: the newest one, when a
   *  leave and a rejoin briefly leave two. */
  private findSelf(): number | null {
    const table = this.shard?.latest;
    if (!table || !this.avatarId) return null;
    let found: number | null = null;
    for (const e of table.entities.values()) {
      if (this.avatarIdOf(e.id, e.components) === this.avatarId && (found === null || e.id > found)) {
        found = e.id;
      }
    }
    return found;
  }

  private avatarIdOf(entity: number, components: ReadonlyMap<number, Uint8Array> | undefined): string | null {
    const bytes = components?.get(AVATAR);
    if (!bytes) return null;
    const cached = this.avatarOf.get(entity);
    if (cached && cached.bytes === bytes) return cached.id;
    const id = this.decoder.decode(bytes);
    this.avatarOf.set(entity, { bytes, id });
    return id;
  }

  /**
   * After each frame: the shard's position for us plus the moves it has
   * not processed is where we should be. If the local player is somewhere
   * else, the shard clamped a move: follow it.
   */
  private reconcile(ack: number) {
    const shard = this.shard;
    const me = this.me;
    if (!shard || !me) return;
    this.selfEntity = this.findSelf();
    const mine = this.selfEntity === null ? undefined : shard.latest.get(this.selfEntity);
    if (!mine) {
      this.selfHealth = null;
      return;
    }
    this.lastSeenSelfAt = performance.now() / 1000;
    this.selfHealth = mine.components.get(HEALTH)?.[0] ?? 100;
    const predicted = me.reconcile({ x: mine.x, y: mine.y, z: mine.z }, ack);
    if (this.dead) return;
    const dx = predicted.x - this.lastSent.x;
    const dy = predicted.y - this.lastSent.y;
    const dz = predicted.z - this.lastSent.z;
    const off = Math.hypot(dx, dy, dz);
    if (off > TELEPORT_M) {
      // Where the shard put us (a spawn point, or a rejoin): go there.
      this.player.teleport(new THREE.Vector3(predicted.x, predicted.y, predicted.z));
      this.lastSent.copy(this.player.position);
    } else if (off > CORRECT_M) {
      this.player.position.x += dx;
      this.player.position.y += dy;
      this.player.position.z += dz;
      this.lastSent.set(predicted.x, predicted.y, predicted.z);
    }
  }

  update(ctx: FrameCtx) {
    const shard = this.shard;
    if (shard) {
      shard.frame();
      for (const id of shard.left) this.avatarOf.delete(id);
      this.sendMove(shard, ctx.time);
      this.collectRemotes(shard);
    }

    // Destruction sync — batch keys, one in-flight call at a time.
    if (this.pendingKeys.length > 0 && !this.flushInFlight) {
      const batch = this.pendingKeys.splice(0, 64);
      this.flushInFlight = true;
      this.track(callFn("destroyBlocks", { keys: batch }))
        .catch(() => {
          // Re-queue on failure so destruction never silently desyncs.
          this.pendingKeys.unshift(...batch);
        })
        .finally(() => {
          this.flushInFlight = false;
        });
    }
  }

  private sendMove(shard: ShardGame<IslandInput>, time: number) {
    if (!shard.connected) return;
    const now = performance.now() / 1000;
    // The shard dropped us (idle, or restarted): join again.
    if (
      this.selfEntity === null ||
      (now - this.lastSeenSelfAt > REJOIN_S && now - this.lastJoinAt > REJOIN_S)
    ) {
      if (now - this.lastJoinAt > REJOIN_S) this.join();
      if (this.selfEntity === null) return;
    }
    if (time - this.lastSentAt < 1 / SEND_HZ) return;
    const p = this.player.position;
    const heading = this.player.yaw;
    const pitch = this.player.pitch;
    const moved =
      !this.dead &&
      (p.distanceToSquared(this.lastSent) > 1e-4 ||
        Math.abs(heading - this.lastLook.heading) > 0.01 ||
        Math.abs(pitch - this.lastLook.pitch) > 0.02);
    if (!moved && time - this.lastSentAt < HEARTBEAT_S) return;
    // The dead do not move; their moves only keep them on the island.
    const d = this.dead ? { x: 0, y: 0, z: 0 } : p.clone().sub(this.lastSent);
    const seq = shard.send({ move: { dx: d.x, dy: d.y, dz: d.z, heading, pitch } });
    if (seq === 0) return;
    this.lastSentAt = time;
    this.lastLook = { heading, pitch };
    if (!this.dead) this.lastSent.copy(p);
  }

  private collectRemotes(shard: ShardGame<IslandInput>) {
    this.remotes.length = 0;
    this.entityOf.clear();
    // A leave and a join close together leave two entities for one avatar
    // for the interpolation delay: draw the newer (ids only grow).
    const newest = new Map<string, InterpolatedEntity>();
    for (const e of shard.entities.values()) {
      const avatarId = this.avatarIdOf(e.id, e.components);
      if (!avatarId) continue;
      const seen = newest.get(avatarId);
      if (!seen || e.id > seen.id) newest.set(avatarId, e);
    }
    let i = 0;
    for (const [avatarId, e] of newest) {
      this.entityOf.set(avatarId, e.id);
      if (avatarId === this.avatarId) continue;
      const pose = this.pool[i] ?? (this.pool[i] = {} as RemotePose);
      i++;
      this.fill(pose, avatarId, e);
      this.remotes.push(pose);
    }
  }

  private fill(pose: RemotePose, avatarId: string, e: InterpolatedEntity) {
    const from = readLook(e.from.components.get(LOOK));
    const to = readLook(e.to.components.get(LOOK));
    pose.avatarId = avatarId;
    pose.entity = e.id;
    pose.x = e.x;
    pose.y = e.y;
    pose.z = e.z;
    pose.heading = lerpAngle(from.heading, to.heading, e.t);
    pose.pitch = from.pitch + (to.pitch - from.pitch) * e.t;
    pose.health = e.components.get(HEALTH)?.[0] ?? 100;
  }

  dispose() {
    this.pendingKeys = [];
    if (this.touchTimer) clearInterval(this.touchTimer);
    this.shard?.send("leave");
    this.shard?.close();
    this.shard = null;
  }
}
