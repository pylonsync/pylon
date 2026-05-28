// OpQueue — single serialized channel for the engine's outbound
// network operations (pull, push, reconcile, refresh, resetReplica).
//
// Why this exists: before unification, each op had its own ad-hoc
// dedupe primitive (inFlightPush, inFlightReconcile, "void refresh()"
// fire-and-forget). The combinations leaked races — pull mid-refresh,
// reconcile mid-pull, session-changed-mid-reconcile — that each
// required their own inline guard. Putting every op on one FIFO
// reduces the race surface to "did a prior op execute between when I
// scheduled and when I ran" — which is just a counter check, not a
// snapshot comparison.
//
// Ops are keyed; concurrent enqueues with the same key share the
// running promise. That preserves the old "callers always get the
// same promise while X is running" coalescing semantics without
// needing a separate inFlightX field per op.
//
// Apply queue (change-event applies) stays SEPARATE. WS events that
// arrive while a pull is in flight should land in the local replica
// without waiting for the pull's HTTP round-trip. That separation is
// the only reason we don't put `applyChanges` on this queue too.

export class OpQueue {
  private chain: Promise<void> = Promise.resolve();
  private pending = new Map<string, Promise<unknown>>();
  /** Monotonic op counter — incremented as each op begins running.
   *  Lets a caller stash a snapshot ("epoch when I scheduled") and
   *  detect after `await` whether another op ran in between. */
  private epoch = 0;

  /** Schedule an op behind any currently running/queued ops.
   *
   *  Coalescing: if `key` matches a pending op, the existing promise
   *  is returned — the new caller does NOT re-enqueue. This preserves
   *  the "many concurrent callers share one in-flight op" semantics
   *  that pull/push/reconcile had via their respective mutexes. */
  enqueue<T>(key: string, fn: () => Promise<T>): Promise<T> {
    const existing = this.pending.get(key);
    if (existing) return existing as Promise<T>;

    let resolveOuter!: (v: T) => void;
    let rejectOuter!: (e: unknown) => void;
    const outer = new Promise<T>((res, rej) => {
      resolveOuter = res;
      rejectOuter = rej;
    });
    this.pending.set(key, outer);

    const prev = this.chain;
    this.chain = prev.then(async () => {
      this.epoch += 1;
      try {
        const v = await fn();
        resolveOuter(v);
      } catch (e) {
        rejectOuter(e);
      } finally {
        // Clear key BEFORE outer settles so a downstream `.then()` on
        // the returned promise can re-enqueue without colliding with
        // our own pending entry.
        this.pending.delete(key);
      }
    });

    return outer;
  }

  /** Current epoch — read before scheduling, compare after `await` to
   *  detect "another op ran while I was waiting." */
  currentEpoch(): number {
    return this.epoch;
  }
}
