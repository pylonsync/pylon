/**
 * Entity interpolation for render loops.
 *
 * Record every replication frame with the tick it was built on; each render
 * frame, call `update` with a tick a little behind the server's (see
 * `ShardClock`). Each entity is then drawn between the two samples around
 * that tick, so motion is smooth even though frames arrive at the tick rate
 * and with jitter. Spawns, despawns, and component changes appear at the
 * tick they happened, in step with the positions.
 */

import type { EntityTable, ReplicatedEntity, ReplicationSummary } from "./replication";

export interface InterpolationOptions {
  /**
   * A move longer than this (world units) between two samples is a
   * teleport: the entity jumps instead of sliding. Default: no limit.
   */
  snapDistance?: number;
  /**
   * Treat an entity that sent no update as not moving, so its next move
   * starts from the tick before it arrives. Default true. Set false for a
   * shard with a byte budget (`max_bytes_per_tick`), where updates can
   * wait for later ticks.
   */
  holdWhenQuiet?: boolean;
  /** Samples kept per entity. Default 32. */
  maxSamples?: number;
}

/** An entity's state at one tick. */
export interface EntitySample {
  readonly tick: number;
  readonly x: number;
  readonly y: number;
  readonly z: number;
  /** Component id to bytes. Shared between samples; do not change it. */
  readonly components: ReadonlyMap<number, Uint8Array>;
}

/** One entity as drawn at the render tick. The object is reused. */
export interface InterpolatedEntity {
  readonly id: number;
  x: number;
  y: number;
  z: number;
  /** Components as of the sample at or before the render tick. */
  components: ReadonlyMap<number, Uint8Array>;
  /**
   * The samples around the render tick and the fraction between them, to
   * interpolate game data kept in components (a heading, a health bar).
   * `from` and `to` are the same sample when there is no later one.
   */
  from: EntitySample;
  to: EntitySample;
  t: number;
}

/** One entity from spawn to despawn. An id can have a later life. */
interface Life {
  spawnTick: number;
  /** The tick it despawned on, or null while it exists. */
  despawnTick: number | null;
  samples: EntitySample[];
}

const EMPTY: ReadonlyMap<number, Uint8Array> = new Map();
/** Ticks an ended life is kept for a render tick that lags behind. */
const ENDED_TICKS = 256;

export class EntityInterpolator {
  /** Entities that exist at the last `update`'s render tick. */
  readonly entities = new Map<number, InterpolatedEntity>();
  /**
   * Ids the last `update` added to `entities`. An id in both `left` and
   * `entered` names a new entity that reused the id: rebuild what you
   * drew for it.
   */
  readonly entered: number[] = [];
  /** Ids the last `update` removed from `entities`. */
  readonly left: number[] = [];

  private readonly lives = new Map<number, Life[]>();
  /** The life each entry in `entities` shows. */
  private readonly shown = new Map<number, Life>();
  private readonly snapDistance: number;
  private readonly holdWhenQuiet: boolean;
  private readonly maxSamples: number;
  private lastTick = -1;
  /** The newest tick the last `update` drew; a render tick never goes
   *  below it. */
  private floor = -Infinity;
  /** Lives that ended, oldest first, so `record` can drop them when
   *  `update` does not run (a hidden tab keeps receiving frames). */
  private ended: Array<{ id: number; life: Life }> = [];

  constructor(options: InterpolationOptions = {}) {
    this.snapDistance = options.snapDistance ?? Infinity;
    this.holdWhenQuiet = options.holdWhenQuiet ?? true;
    this.maxSamples = Math.max(2, options.maxSamples ?? 32);
  }

  /** The newest tick recorded, or -1. */
  get latestTick(): number {
    return this.lastTick;
  }

  /** Forget everything. The next `update` removes every entity. */
  clear(): void {
    this.lives.clear();
    this.ended = [];
    this.lastTick = -1;
    this.floor = -Infinity;
  }

  /**
   * Record the table after a replication frame for `tick` applied, with the
   * summary `EntityTable.apply` returned.
   */
  record(table: EntityTable, summary: ReplicationSummary, tick: number): void {
    if (tick < this.lastTick) {
      // Ticks went back: a restarted or different shard.
      this.clear();
    }
    this.lastTick = tick;
    this.dropEnded(tick);

    if (summary.full) {
      // The table was rebuilt: what it lacks now is gone.
      const present = new Set(summary.spawned);
      for (const [id, lives] of this.lives) {
        const open = lives[lives.length - 1];
        if (open.despawnTick === null && !present.has(id)) this.end(id, open, tick);
      }
      for (const id of summary.spawned) {
        const e = table.get(id);
        if (!e) continue;
        const open = this.openLife(id);
        if (open) this.push(open, e, tick);
        else this.spawn(id, e, tick);
      }
      return;
    }

    for (const id of summary.despawned) {
      const open = this.openLife(id);
      if (open) this.end(id, open, tick);
    }
    for (const id of summary.spawned) {
      const e = table.get(id);
      if (e) this.spawn(id, e, tick);
    }
    for (const id of summary.updated) {
      const e = table.get(id);
      const open = this.openLife(id);
      if (e && open) this.push(open, e, tick);
    }
  }

  /**
   * Place every entity at `renderTick` (fractional). A render tick below
   * one already drawn is raised to it, so nothing drawn goes back in time.
   */
  update(renderTick: number): void {
    this.entered.length = 0;
    this.left.length = 0;
    renderTick = Math.max(renderTick, this.floor);
    // What this update draws is at most the newest tick recorded.
    this.floor = Math.min(renderTick, this.lastTick);
    for (const [id, lives] of this.lives) {
      // Lives that ended by the render tick are over.
      while (lives.length > 0 && lives[0].despawnTick !== null && lives[0].despawnTick <= renderTick) {
        lives.shift();
      }
      if (lives.length === 0) {
        this.lives.delete(id);
        this.remove(id);
        continue;
      }
      const life = lives[0];
      if (life.spawnTick > renderTick) {
        // Not spawned yet at the render tick.
        this.remove(id);
        continue;
      }
      this.place(id, life, renderTick);
    }
    for (const id of this.entities.keys()) {
      if (!this.lives.has(id)) this.remove(id);
    }
  }

  private end(id: number, life: Life, tick: number): void {
    life.despawnTick = tick;
    this.ended.push({ id, life });
  }

  /**
   * Forget lives that ended long before `tick`. `update` removes them
   * when it runs; this bounds them when it does not.
   */
  private dropEnded(tick: number): void {
    let n = 0;
    while (n < this.ended.length && (this.ended[n].life.despawnTick as number) < tick - ENDED_TICKS) {
      const { id, life } = this.ended[n];
      const lives = this.lives.get(id);
      if (lives) {
        const i = lives.indexOf(life);
        if (i >= 0) lives.splice(i, 1);
        if (lives.length === 0) this.lives.delete(id);
      }
      n++;
    }
    if (n > 0) this.ended.splice(0, n);
  }

  private openLife(id: number): Life | null {
    const lives = this.lives.get(id);
    const last = lives?.[lives.length - 1];
    return last && last.despawnTick === null ? last : null;
  }

  private spawn(id: number, e: ReplicatedEntity, tick: number): void {
    let lives = this.lives.get(id);
    if (!lives) {
      lives = [];
      this.lives.set(id, lives);
    }
    const open = lives[lives.length - 1];
    if (open && open.despawnTick === null) this.end(id, open, tick);
    lives.push({
      spawnTick: tick,
      despawnTick: null,
      samples: [{ tick, x: e.x, y: e.y, z: e.z, components: new Map(e.components) }],
    });
  }

  private push(life: Life, e: ReplicatedEntity, tick: number): void {
    const samples = life.samples;
    const last = samples[samples.length - 1];
    const components = sameComponents(last.components, e.components)
      ? last.components
      : new Map(e.components);
    const moved = last.x !== e.x || last.y !== e.y || last.z !== e.z;
    if (!moved && components === last.components) return;
    if (last.tick === tick) {
      samples[samples.length - 1] = { tick, x: e.x, y: e.y, z: e.z, components };
      return;
    }
    if (this.holdWhenQuiet && moved && last.tick < tick - 1) {
      // No update since `last`: it stood still until the tick before.
      samples.push({ ...last, tick: tick - 1 });
    }
    samples.push({ tick, x: e.x, y: e.y, z: e.z, components });
    while (samples.length > this.maxSamples) samples.shift();
  }

  private place(id: number, life: Life, renderTick: number): void {
    const samples = life.samples;
    // The last sample at or before the render tick. Samples before the
    // spawn tick do not exist, and the first sample is the spawn.
    let i = samples.length - 1;
    while (i > 0 && samples[i].tick > renderTick) i--;
    const from = samples[i];
    const to = samples[i + 1] ?? from;
    let t = to === from ? 0 : (renderTick - from.tick) / (to.tick - from.tick);
    t = Math.min(1, Math.max(0, t));
    const dx = to.x - from.x;
    const dy = to.y - from.y;
    const dz = to.z - from.z;
    const teleport = dx * dx + dy * dy + dz * dz > this.snapDistance * this.snapDistance;
    const k = teleport ? 0 : t;

    let view = this.entities.get(id);
    if (view && this.shown.get(id) !== life) {
      // The id names a new entity now: the old one left.
      this.remove(id);
      view = undefined;
    }
    if (!view) {
      view = { id, x: 0, y: 0, z: 0, components: EMPTY, from, to, t };
      this.entities.set(id, view);
      this.shown.set(id, life);
      this.entered.push(id);
    }
    view.x = from.x + dx * k;
    view.y = from.y + dy * k;
    view.z = from.z + dz * k;
    view.components = from.components;
    view.from = from;
    view.to = to;
    view.t = t;

    // Samples two behind the one in use are no longer needed.
    if (i > 1) samples.splice(0, i - 1);
  }

  private remove(id: number): void {
    this.shown.delete(id);
    if (this.entities.delete(id)) this.left.push(id);
  }
}

/** True when `a` and `b` hold the same components with the same bytes. */
function sameComponents(
  a: ReadonlyMap<number, Uint8Array>,
  b: ReadonlyMap<number, Uint8Array>,
): boolean {
  if (a.size !== b.size) return false;
  for (const [k, v] of b) {
    const w = a.get(k);
    if (w === v) continue;
    if (!w || w.length !== v.length) return false;
    for (let i = 0; i < v.length; i++) if (w[i] !== v[i]) return false;
  }
  return true;
}
