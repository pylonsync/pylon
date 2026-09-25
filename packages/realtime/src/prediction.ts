/**
 * Client-side prediction for the local player.
 *
 * Apply each input locally as it is sent, and keep it until the shard
 * acknowledges it. When a frame arrives with the server's state and its
 * `ack` (the highest input sequence number the shard has processed), start
 * from the server's state and apply the inputs it has not processed yet.
 * The result is where the local player is after every input, corrected by
 * whatever the server decided.
 *
 * `step` must be deterministic and must match what the shard does with an
 * input closely enough that the replayed state agrees with the server's.
 * It must not change `state` in place; return a new one.
 */

export interface PredictorOptions {
  /**
   * Inputs kept while waiting for an ack. Past this the oldest are
   * dropped (the prediction then misses them). Default 256.
   */
  maxPending?: number;
}

interface Pending<I> {
  seq: number;
  input: I;
}

export class Predictor<S, I> {
  private pending: Array<Pending<I>> = [];
  private readonly maxPending: number;

  constructor(
    private readonly step: (state: S, input: I) => S,
    options: PredictorOptions = {},
  ) {
    this.maxPending = Math.max(1, options.maxPending ?? 256);
  }

  /** Inputs sent and not yet acknowledged. */
  get size(): number {
    return this.pending.length;
  }

  /** Record an input sent with sequence number `seq`. A `seq` of 0 (not
   *  sent) is ignored. */
  push(seq: number, input: I): void {
    if (seq <= 0) return;
    this.pending.push({ seq, input });
    if (this.pending.length > this.maxPending) this.pending.shift();
  }

  /** The shard refused input `seq`: it will never apply. */
  reject(seq: number | null): void {
    if (seq === null) return;
    this.pending = this.pending.filter((p) => p.seq !== seq);
  }

  /**
   * The predicted state: `server` (the shard's state as of a frame) with
   * every input after `ack` applied. Inputs up to `ack` are dropped.
   */
  reconcile(server: S, ack: number): S {
    let drop = 0;
    while (drop < this.pending.length && this.pending[drop].seq <= ack) drop++;
    if (drop > 0) this.pending.splice(0, drop);
    let state = server;
    for (const p of this.pending) state = this.step(state, p.input);
    return state;
  }

  /**
   * Drop every pending input. Call it when the connection reopens: inputs
   * sent on the old connection either applied before it closed (and are
   * in the server's state) or never will.
   */
  reset(): void {
    this.pending = [];
  }
}
