/**
 * A shard client for a game's render loop: the connection, the clock,
 * interpolated entities, and prediction, with no framework and no
 * re-render per frame.
 *
 * ```ts
 * const game = connectShardGame("zone-1", { subscriberId, ticket, tickRate: 20 });
 * const me = game.predict<Vec3>((p, input) => move(p, input));
 *
 * function frame(now: number) {
 *   game.frame(now); // places game.entities at the render tick
 *   for (const e of game.entities.values()) draw(e.id, e.x, e.y, e.z);
 *   requestAnimationFrame(frame);
 * }
 * requestAnimationFrame(frame);
 *
 * game.onReplication((table, _summary, _tick, ack) => {
 *   const mine = table.get(myEntityId);
 *   if (mine) local = me.reconcile({ x: mine.x, y: mine.y, z: mine.z }, ack);
 * });
 * onKey((input) => {
 *   // 0: not sent (the connection is down), so do not predict it.
 *   if (game.send(input)) local = move(local, input);
 * });
 * ```
 */

import type { ShardClock } from "./clock";
import { connectShard, type ShardClient, type ShardConnectOptions } from "./connection";
import {
  EntityInterpolator,
  type InterpolatedEntity,
  type InterpolationOptions,
} from "./interpolation";
import { Predictor, type PredictorOptions } from "./prediction";
import type { EntityTable, ReplicationSummary } from "./replication";
import type { ShardInputRejection } from "./wire";

export interface ShardGameOptions extends ShardConnectOptions {
  /**
   * How far behind the shard's estimated current tick entities are drawn,
   * in ms. It must cover the time between frames plus network jitter.
   * Default 100.
   */
  interpolationDelayMs?: number;
  interpolation?: InterpolationOptions;
}

export interface ShardGame<TInput = unknown> {
  /** The underlying connection. */
  readonly connection: ShardClient<unknown, TInput>;
  readonly clock: ShardClock;
  /** Entities placed at the render tick by the last `frame` call. */
  readonly entities: ReadonlyMap<number, InterpolatedEntity>;
  /** Ids the last `frame` added to and removed from `entities`. */
  readonly entered: readonly number[];
  readonly left: readonly number[];
  /** The newest state the shard sent, not interpolated. */
  readonly latest: EntityTable;
  readonly tick: number;
  readonly ack: number;
  readonly rttMs: number | null;
  readonly connected: boolean;
  /**
   * Place `entities` for a render frame at `now` (ms, default
   * `performance.now()`). Returns the render tick, or -1 before the first
   * frame.
   */
  frame(now?: number): number;
  /** Send an input; every predictor made by `predict` records it. Returns
   *  its sequence number, or 0 when it was not sent. */
  send(input: TInput): number;
  /**
   * A predictor that records every input `send` sends, forgets inputs the
   * shard refuses, and resets when the connection reopens. Call
   * `reconcile` with the local entity's server state after each frame.
   */
  predict<S>(step: (state: S, input: TInput) => S, options?: PredictorOptions): Predictor<S, TInput>;
  onReplication(
    fn: (entities: EntityTable, summary: ReplicationSummary, tick: number, ack: number) => void,
  ): void;
  onInputRejected(fn: (rejection: ShardInputRejection) => void): void;
  onOpen(fn: () => void): void;
  onClose(fn: () => void): void;
  onError(fn: (err: Error) => void): void;
  close(): void;
}

/** Connect to a replicating shard for a render loop. */
export function connectShardGame<TInput = unknown>(
  shardId: string,
  options: ShardGameOptions,
): ShardGame<TInput> {
  const now = options.now ?? (() => performance.now());
  const connection = connectShard<unknown, TInput>(shardId, { ...options, now });
  const interpolator = new EntityInterpolator(options.interpolation);
  const delayMs = options.interpolationDelayMs ?? 100;
  const predictors: Array<Predictor<unknown, TInput>> = [];

  connection.onReplication((table, summary, tick) => interpolator.record(table, summary, tick));
  connection.onInputRejected((r) => {
    for (const p of predictors) p.reject(r.clientSeq);
  });
  connection.onOpen(() => {
    for (const p of predictors) p.reset();
  });

  return {
    connection,
    get clock() {
      return connection.clock;
    },
    get entities() {
      return interpolator.entities;
    },
    get entered() {
      return interpolator.entered;
    },
    get left() {
      return interpolator.left;
    },
    get latest() {
      return connection.entities;
    },
    get tick() {
      return connection.tick;
    },
    get ack() {
      return connection.ack;
    },
    get rttMs() {
      return connection.rttMs;
    },
    get connected() {
      return connection.connected;
    },
    frame(at = now()) {
      const clock = connection.clock;
      if (!clock.ready) return -1;
      const renderTick = clock.serverTick(at) - delayMs / clock.tickMs;
      interpolator.update(renderTick);
      return renderTick;
    },
    send(input) {
      const seq = connection.send(input);
      for (const p of predictors) p.push(seq, input);
      return seq;
    },
    predict<S>(step: (state: S, input: TInput) => S, predictorOptions?: PredictorOptions) {
      const p = new Predictor<S, TInput>(step, predictorOptions);
      predictors.push(p as unknown as Predictor<unknown, TInput>);
      return p;
    },
    onReplication: (fn) => connection.onReplication(fn),
    onInputRejected: (fn) => connection.onInputRejected(fn),
    onOpen: (fn) => connection.onOpen(fn),
    onClose: (fn) => connection.onClose(fn),
    onError: (fn) => connection.onError(fn),
    close: () => connection.close(),
  };
}
