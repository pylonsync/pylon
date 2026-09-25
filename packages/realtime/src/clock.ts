/**
 * An estimate of the shard's current tick, from the ticks in the frames it
 * sends and when they arrive.
 *
 * Each frame carries the tick it was built on. The frame that arrived
 * soonest after it was built (lowest delay) anchors the estimate, so
 * network jitter only ever makes a frame look late. The tick length is the
 * shard's tick rate when the client knows it, or else a least-squares fit
 * of arrival time against tick.
 *
 * The estimate is continuous: when a new frame moves it, the change is
 * spread over time (at most `slew` of real time) instead of jumping, so a
 * render loop that samples it never goes backwards. A change larger than
 * `snapMs` (a stall, a shard restart) is applied at once. After three
 * frames in a row arrive more than `snapMs` late, the older frames are
 * dropped, so a shard that stalled is followed on its new schedule.
 */

export interface ShardClockOptions {
  /** The shard's tick rate in ticks per second, when the client knows it. */
  tickRate?: number;
  /** Frames the estimate uses. Default 64. */
  window?: number;
  /** Fraction of real time the estimate may speed up or slow down by to
   *  absorb a correction. Default 0.05. */
  slew?: number;
  /** A correction larger than this (ms) applies at once. Default 250. */
  snapMs?: number;
}

interface Sample {
  tick: number;
  at: number;
}

const DEFAULT_TICK_MS = 50;
/** Late frames in a row that mean the schedule moved. */
const LATE_FRAMES = 3;

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

    // The shard's schedule moved later (it stalled, or the path got
    // slower for good) when several frames in a row are late by more than
    // `snapMs`. One late frame is jitter. The samples from before the move
    // would anchor the estimate too early, so they go.
    if (this.ready) {
      const expected = this.anchorAt + (tick - this.anchorTick) * this.tickMs;
      this.late = at - expected > this.snapMs ? this.late + 1 : 0;
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
      // Keep what a render loop sees continuous; decay the difference.
      this.offset = shown - this.raw(at);
      if (Math.abs(this.offset) * tickMs > this.snapMs) this.offset = 0;
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
      const step = (elapsed * this.slew) / this.tickMs;
      this.offset -= Math.max(-step, Math.min(step, this.offset));
      this.slewedTo = now;
    }
    return this.raw(now) + this.offset;
  }

  private raw(now: number): number {
    return this.anchorTick + (now - this.anchorAt) / this.tickMs;
  }

  /** Least-squares slope of arrival time against tick. */
  private fitTickMs(): void {
    const n = this.samples.length;
    if (n < 3) return;
    const first = this.samples[0];
    const span = this.samples[n - 1].tick - first.tick;
    if (span < 4) return;
    let sx = 0;
    let sy = 0;
    for (const s of this.samples) {
      sx += s.tick - first.tick;
      sy += s.at - first.at;
    }
    const mx = sx / n;
    const my = sy / n;
    let sxy = 0;
    let sxx = 0;
    for (const s of this.samples) {
      const dx = s.tick - first.tick - mx;
      sxy += dx * (s.at - first.at - my);
      sxx += dx * dx;
    }
    if (sxx === 0) return;
    const slope = sxy / sxx;
    if (Number.isFinite(slope)) this.measuredTickMs = Math.min(2000, Math.max(1, slope));
  }
}
