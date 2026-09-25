/**
 * An estimate of the shard's current tick, from the ticks in the frames it
 * sends and when they arrive.
 *
 * Each frame carries the tick it was built on. The frame that arrived
 * soonest after it was built (lowest delay) anchors the estimate, so
 * network jitter only ever makes a frame look late. The tick length is the
 * shard's tick rate when the client knows it, or else the median slope of
 * arrival time against tick.
 *
 * The estimate is continuous: when a new frame moves it, the change is
 * spread over time instead of jumping (from `slew` of real time for small
 * changes up to all of it). A step forward larger than `snapMs` applies at
 * once. A step back larger than that applies only as far as the newest
 * tick received, so it never goes back past a tick a frame carried.
 *
 * After three frames in a row arrive more than `snapMs` late and spaced
 * like ticks (a shard that stalled), the older frames are dropped and the
 * clock follows the new schedule. Frames that arrive together (a network
 * burst) do not count.
 */

export interface ShardClockOptions {
  /** The shard's tick rate in ticks per second, when the client knows it. */
  tickRate?: number;
  /** Frames the estimate uses. Default 64. */
  window?: number;
  /** The least fraction of real time the estimate speeds up or slows
   *  down by to absorb a correction. Default 0.05. */
  slew?: number;
  /** A step forward larger than this (ms) applies at once, and a
   *  correction this large or larger runs at full rate. Default 250. */
  snapMs?: number;
}

interface Sample {
  tick: number;
  at: number;
}

const DEFAULT_TICK_MS = 50;
/** Late frames in a row that mean the schedule moved. */
const LATE_FRAMES = 3;
/** Samples the tick length is measured from; fewer keep the last value. */
const MIN_FIT_SAMPLES = 8;

export class ShardClock {
  private readonly fixedTickMs: number | null;
  private readonly window: number;
  private readonly slew: number;
  private readonly snapMs: number;
  private samples: Sample[] = [];
  private measuredTickMs = DEFAULT_TICK_MS;
  /** The newest tick seen, and the estimated arrival time of a frame for
   *  it with the lowest delay seen. */
  private anchorTick = -1;
  private anchorAt = 0;
  /** Ticks the displayed estimate is ahead of the raw one; decays to 0. */
  private offset = 0;
  private slewedTo = 0;
  /** Frames in a row that arrived more than `snapMs` after the anchor
   *  predicted. */
  private late = 0;

  constructor(options: ShardClockOptions = {}) {
    const rate = options.tickRate;
    this.fixedTickMs = rate !== undefined && rate > 0 && Number.isFinite(rate) ? 1000 / rate : null;
    this.window = Math.max(2, options.window ?? 64);
    this.slew = Math.min(0.5, Math.max(0, options.slew ?? 0.05));
    this.snapMs = options.snapMs ?? 250;
  }

  /** True once a frame has arrived. */
  get ready(): boolean {
    return this.anchorTick >= 0;
  }

  /** The newest tick a frame carried, or -1. */
  get latestTick(): number {
    return this.anchorTick;
  }

  /** Milliseconds per tick: the configured rate, or the measured one. */
  get tickMs(): number {
    return this.fixedTickMs ?? this.measuredTickMs;
  }

  /** Forget everything (a new shard). */
  reset(): void {
    this.samples = [];
    this.measuredTickMs = DEFAULT_TICK_MS;
    this.anchorTick = -1;
    this.anchorAt = 0;
    this.offset = 0;
    this.slewedTo = 0;
    this.late = 0;
  }

  /** Record a frame for `tick` that arrived at `at` (ms, monotonic). */
  observe(tick: number, at: number): void {
    if (tick < this.anchorTick) {
      // Ticks went back: the shard restarted, or this is another shard.
      this.reset();
    }
    const shown = this.ready ? this.serverTick(at) : null;
    const newestBefore = this.anchorTick;

    // The shard's schedule moved later (it stalled, or the path got
    // slower for good) when several frames in a row arrive late by more
    // than `snapMs`, spaced like ticks. One late frame is jitter, and
    // frames that arrive together are a network burst: the shard kept its
    // schedule. After a move, the samples from before it would anchor the
    // estimate too early, so they go.
    const prev = this.samples[this.samples.length - 1];
    if (this.ready && prev && tick > prev.tick) {
      const expected = this.anchorAt + (tick - this.anchorTick) * this.tickMs;
      const spaced = at - prev.at >= 0.5 * this.tickMs * (tick - prev.tick);
      this.late = spaced && at - expected > this.snapMs ? this.late + 1 : 0;
      if (this.late >= LATE_FRAMES) {
        this.samples = this.samples.slice(-(LATE_FRAMES - 1));
        this.late = 0;
      }
    }

    const last = this.samples[this.samples.length - 1];
    if (last && last.tick === tick) {
      // Another frame for the same tick: keep the earlier arrival.
      if (at >= last.at) return;
      last.at = at;
    } else {
      this.samples.push({ tick, at });
      if (this.samples.length > this.window) this.samples.shift();
    }
    if (this.fixedTickMs === null) this.fitTickMs();

    const tickMs = this.tickMs;
    let anchorAt = Infinity;
    for (const s of this.samples) {
      anchorAt = Math.min(anchorAt, s.at + (tick - s.tick) * tickMs);
    }
    this.anchorTick = tick;
    this.anchorAt = anchorAt;

    if (shown === null) {
      this.offset = 0;
    } else {
      // Keep what a render loop sees continuous; `serverTick` decays the
      // difference. A large step forward applies at once. A large step
      // back applies only down to the newest tick received before this
      // frame: past it the estimate had run ahead of every frame (a shard
      // that stalled), so nothing drawn goes back.
      const raw = this.raw(at);
      this.offset = shown - raw;
      if (Math.abs(this.offset) * tickMs > this.snapMs) {
        this.offset = this.offset > 0 ? Math.max(0, Math.min(shown, newestBefore) - raw) : 0;
      }
    }
    this.slewedTo = Math.max(this.slewedTo, at);
  }

  /**
   * The estimated tick the shard is on at `now` (ms, same clock as
   * `observe`), with a fraction. Before the first frame, -1.
   */
  serverTick(now: number): number {
    if (!this.ready) return -1;
    const elapsed = now - this.slewedTo;
    if (elapsed > 0) {
      // The correction runs faster the larger it is, from `slew` up to
      // all of real time (the estimate stands still, or runs at double
      // speed).
      const offsetMs = Math.abs(this.offset) * this.tickMs;
      const rate = Math.min(1, Math.max(this.slew, offsetMs / this.snapMs));
      const step = (elapsed * rate) / this.tickMs;
      this.offset -= Math.max(-step, Math.min(step, this.offset));
      this.slewedTo = now;
    }
    return this.raw(now) + this.offset;
  }

  private raw(now: number): number {
    return this.anchorTick + (now - this.anchorAt) / this.tickMs;
  }

  /**
   * The median slope of arrival time against tick over every pair of
   * samples (Theil-Sen): frames that arrive together in a burst are a
   * minority of pairs and do not pull it.
   */
  private fitTickMs(): void {
    const n = this.samples.length;
    if (n < MIN_FIT_SAMPLES) return;
    if (this.samples[n - 1].tick - this.samples[0].tick < 4) return;
    const slopes: number[] = [];
    for (let i = 0; i < n; i++) {
      for (let j = i + 1; j < n; j++) {
        const dt = this.samples[j].tick - this.samples[i].tick;
        if (dt > 0) slopes.push((this.samples[j].at - this.samples[i].at) / dt);
      }
    }
    if (slopes.length === 0) return;
    slopes.sort((a, b) => a - b);
    const slope = slopes[slopes.length >> 1];
    if (Number.isFinite(slope)) this.measuredTickMs = Math.min(2000, Math.max(1, slope));
  }
}
