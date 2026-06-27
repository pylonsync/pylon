/**
 * Network glue between the game loop and Pylon.
 *
 * Outbound only — inbound state (other avatars, destruction rows)
 * flows through React live queries in page.tsx and is pushed into the
 * game via setters. This module throttles pose uploads to ~10 Hz
 * (and skips them entirely while stationary), batches destruction
 * keys, and tracks round-trip latency for the HUD.
 */
import { callFn } from "@pylonsync/react";
import { PLAYER } from "./config";
import type { EventBus, FrameCtx, GameSystem } from "./engine";
import type { Player } from "./player";

function percentile(values: number[], p: number): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const idx = Math.min(sorted.length - 1, Math.floor(sorted.length * p));
  return sorted[idx];
}

export class Net implements GameSystem {
  readonly name = "net";

  private avatarId: string | null = null;
  private lastSentAt = 0;
  // Seeded to Infinity, not NaN: the moved-gate compares |current -
  // lastSent| and every NaN comparison is false, so Infinity makes the
  // first comparison true (the opening pose always sends) and real
  // deltas take over from there.
  private lastSent = {
    x: Infinity,
    y: Infinity,
    z: Infinity,
    heading: Infinity,
    pitch: Infinity,
  };
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
    // Combat: the shooter reports hits; the server clamps damage and
    // range-checks before touching the victim's row.
    events.on("playerHit", ({ avatarId, damage }) => {
      this.track(callFn("damageAvatar", { targetId: avatarId, amount: damage })).catch(() => {
        // Out-of-range / target gone — the world view self-corrects.
      });
    });
    // Death sequence finished locally → heal the row server-side.
    events.on("respawnRequested", () => {
      if (!this.avatarId) return;
      this.track(callFn("respawnAvatar", { avatarId: this.avatarId })).catch(() => {});
    });
  }

  setAvatarId(id: string) {
    this.avatarId = id;
  }

  get hudStats(): { p50: number; p95: number; mutPerSec: number } {
    const now = Date.now();
    this.mutationsInWindow = this.mutationsInWindow.filter((t) => now - t < 1000);
    const recent = this.latencies.slice(-100);
    return {
      p50: Math.round(percentile(recent, 0.5)),
      p95: Math.round(percentile(recent, 0.95)),
      mutPerSec: this.mutationsInWindow.length,
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

  update(ctx: FrameCtx) {
    if (!this.avatarId) return;

    // Pose upload — throttled, and only when the pose moved.
    const interval = 1 / PLAYER.netHz;
    if (ctx.time - this.lastSentAt >= interval) {
      const p = this.player.position;
      const moved =
        Math.abs(p.x - this.lastSent.x) > 0.01 ||
        Math.abs(p.y - this.lastSent.y) > 0.01 ||
        Math.abs(p.z - this.lastSent.z) > 0.01 ||
        Math.abs(this.player.yaw - this.lastSent.heading) > 0.01 ||
        Math.abs(this.player.pitch - this.lastSent.pitch) > 0.02;
      if (moved) {
        this.lastSentAt = ctx.time;
        this.lastSent = {
          x: p.x,
          y: p.y,
          z: p.z,
          heading: this.player.yaw,
          pitch: this.player.pitch,
        };
        this.track(
          callFn("moveAvatar", {
            avatarId: this.avatarId,
            x: p.x,
            y: p.y,
            z: p.z,
            heading: this.player.yaw,
            pitch: this.player.pitch,
          }),
        ).catch(() => {
          // Transient network failure — next tick retries naturally.
        });
      }
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

  dispose() {
    this.pendingKeys = [];
  }
}
