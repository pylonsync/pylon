// SubscriptionCoordinator — owns every "this tab wants live updates"
// subscription, regardless of kind (CRDT row, reactive query) or role
// (this tab is leader vs follower).
//
// Two kinds of subscriptions live here:
//
//   1. CRDT row subs (`subscribeCrdt(entity, rowId)`) — refcounted
//      per-tab because N `useLoroDoc` components on the same row share
//      one server subscription. The first add and last remove ship a
//      WS frame; the in-between calls just bump the count.
//
//   2. Reactive query subs (`subscribeReactive(sub_id, fn_name, args, handler)`)
//      — Convex-shape. The sub_id is per-mount; multiple mounts of the
//      same query get different sub_ids. Each sub_id has at most one
//      handler that fires for inbound `reactive-result` / `reactive-error`.
//
// Leader / follower split:
//
//   - LEADER tabs own a real WebSocket and forward each sub to it via
//     `serverSubs.register`. They also track which FOLLOWER tabs are
//     interested in each key (the forwarder/owner sets), so the WS
//     sub stays alive as long as any tab in the origin still wants it.
//
//   - FOLLOWER tabs don't have a WS. They broadcast `sub-register` /
//     `sub-unregister` envelopes over the BroadcastChannel; the leader
//     picks those up and routes them through `handleForwardedRegister`
//     / `handleForwardedUnregister`. Inbound `reactive-result` lands
//     on the leader's WS, the leader broadcasts `reactive-msg` back
//     across tabs, and `handleReactiveMessage` routes to the local
//     handler on whichever tab originally subscribed.
//
// Lifecycle hooks (called by the engine):
//
//   - `seedFromLocalInterest()` — when this tab settles as leader, register
//     every locally-wanted sub with `serverSubs` so the next WS connect
//     replays them.
//   - `replayForwardedSubs()` — when a fresh leader asks for replay, every
//     follower re-broadcasts its currently-wanted subs.
//   - `scrubPeer(tabId)` — when a peer tab disappears (`bye`), drop its
//     entries from forwarder/owner sets and unregister WS subs that
//     are no longer wanted by anyone.

import type { ServerSubscriptions } from "./server-subscriptions";
import type { ReactiveMessage } from "./types";

/** Sentinel for "this leader tab subscribed locally" — used as a
 *  refcount-bearer key in reactive owner sets so the leader's own
 *  subs don't get conflated with forwarded follower subs. */
const OWN_TAB = "__self__";

/** Engine-side surface the coordinator depends on. Kept narrow so the
 *  coordinator's contract with the engine is explicit. */
export interface SubscriptionCoordinatorContext {
  isLeader(): boolean;
  broadcastToTabs(payload: unknown): void;
}

interface ReactiveSpec {
  fn_name: string;
  args: unknown;
}

export class SubscriptionCoordinator {
  /** Per-row local refcount for CRDT subscriptions — N `useLoroDoc`
   *  consumers on the same `(entity, rowId)` in THIS tab share a
   *  single server subscription. Distinct from `reactiveSubOwners`:
   *  CRDT keys are per-row (many consumers per tab) while reactive
   *  sub_ids are per-consumer-instance (typically one). StrictMode
   *  double-subscribe still bumps the count by one each call; the
   *  matching double-unsubscribe early-returns when the count is
   *  already zero, so the math balances. */
  private crdtSubscribers: Map<string, number> = new Map();

  /** Per-row set of FOLLOWER tabIds that have forwarded a
   *  `sub-register` for this key. Leader-only. Used to:
   *    (a) skip the WS unsubscribe until both `crdtSubscribers`
   *        and this set are empty, and
   *    (b) skip the cross-tab binary broadcast when no follower
   *        cares about CRDT (saves bandwidth when the leader is
   *        the only CRDT consumer). */
  private crdtForwarders: Map<string, Set<string>> = new Map();

  /** Per-sub_id ownership set for reactive subscriptions on the
   *  leader: which tabs (self + forwarders) want this sub alive.
   *  A SET, not a count, so a follower crash + late `sub-unregister`
   *  storm can't underflow the count, and a remount/StrictMode
   *  double-subscribe from one tab still counts as one owner. */
  private reactiveSubOwners: Map<string, Set<string>> = new Map();

  /** Inbound message routing for reactive subscriptions — server pushes
   *  `reactive-result` / `reactive-error` envelopes keyed by sub_id and
   *  the hook's handler lives here. */
  private reactiveHandlers: Map<string, (msg: ReactiveMessage) => void> =
    new Map();

  /** Specs (fn_name + args) for every reactive subscription this tab
   *  has registered, regardless of role. On promotion the new leader
   *  uses these to register with serverSubs; on a leader change while
   *  we stay a follower we use them to re-forward `sub-register` to
   *  the new leader. */
  private wantedReactiveSpecs: Map<string, ReactiveSpec> = new Map();

  constructor(
    private readonly serverSubs: ServerSubscriptions,
    private readonly ctx: SubscriptionCoordinatorContext,
  ) {}

  // ---- CRDT subscriptions ------------------------------------------------

  /** Refcounted CRDT row subscribe. First subscriber for the row sends
   *  the WS frame (leader) or forwards to the leader (follower).
   *  Idempotent at the WS level: re-calling with no intervening
   *  unsubscribe just bumps the count. */
  subscribeCrdt(entity: string, rowId: string): void {
    const key = crdtKey(entity, rowId);
    const prev = this.crdtSubscribers.get(key) ?? 0;
    this.crdtSubscribers.set(key, prev + 1);
    if (prev !== 0) return;
    if (this.ctx.isLeader()) {
      // Leader: only send the WS subscribe if no follower had already
      // forwarded one (in which case the WS sub is already alive).
      const hasFwd = (this.crdtForwarders.get(key)?.size ?? 0) > 0;
      if (!hasFwd) {
        this.serverSubs.register(key, {
          type: "crdt-subscribe",
          entity,
          rowId,
        });
      }
    } else {
      // Follower: ask the leader to subscribe on our behalf. The leader
      // echoes binary frames back over the broadcast channel so our
      // local binaryHandlers fire normally.
      this.ctx.broadcastToTabs({
        type: "sub-register",
        kind: "crdt",
        key,
        entity,
        rowId,
      });
    }
  }

  /** Refcount-aware CRDT row unsubscribe. Last unsubscribe ships the
   *  WS frame; intermediate calls just decrement. Calling more times
   *  than `subscribeCrdt` is a no-op (StrictMode-safe). */
  unsubscribeCrdt(entity: string, rowId: string): void {
    const key = crdtKey(entity, rowId);
    const prev = this.crdtSubscribers.get(key) ?? 0;
    if (prev <= 0) return;
    if (prev > 1) {
      this.crdtSubscribers.set(key, prev - 1);
      return;
    }
    this.crdtSubscribers.delete(key);
    if (this.ctx.isLeader()) {
      const remainingFwd = this.crdtForwarders.get(key)?.size ?? 0;
      if (remainingFwd === 0 && this.serverSubs.has(key)) {
        this.serverSubs.unregister(key, {
          type: "crdt-unsubscribe",
          entity,
          rowId,
        });
      }
    } else {
      this.ctx.broadcastToTabs({
        type: "sub-unregister",
        kind: "crdt",
        key,
        entity,
        rowId,
      });
    }
  }

  // ---- Reactive query subscriptions --------------------------------------

  /** Register a reactive query subscription. The caller-minted `sub_id`
   *  is used by the React hook to dispatch result/error pushes to the
   *  right component.
   *
   *  Idempotent: re-calling with the same `sub_id` replaces the prior
   *  handler + spec. Useful when args change and the hook re-registers
   *  — ServerSubscriptions re-sends on payload change, so the WS sees
   *  the new args. */
  subscribeReactive(
    sub_id: string,
    fn_name: string,
    args: unknown,
    handler: (msg: ReactiveMessage) => void,
  ): void {
    this.reactiveHandlers.set(sub_id, handler);
    this.wantedReactiveSpecs.set(sub_id, { fn_name, args });
    if (!this.ctx.isLeader()) {
      // Follower path: the WS lives on the leader. Forward the spec
      // there; the leader registers with its own serverSubs and echoes
      // inbound envelopes back to us via the channel.
      this.ctx.broadcastToTabs({
        type: "sub-register",
        kind: "reactive",
        key: sub_id,
        sub_id,
        fn_name,
        args,
      });
      return;
    }
    let owners = this.reactiveSubOwners.get(sub_id);
    if (!owners) {
      owners = new Set();
      this.reactiveSubOwners.set(sub_id, owners);
    }
    owners.add(OWN_TAB);
    this.serverSubs.register(sub_id, {
      type: "reactive-subscribe",
      sub_id,
      fn_name,
      args,
    });
  }

  /** Tear down a reactive subscription. No-op for unknown sub_ids —
   *  React StrictMode double-unmount doesn't error. */
  unsubscribeReactive(sub_id: string): void {
    if (!this.reactiveHandlers.has(sub_id)) return;
    this.reactiveHandlers.delete(sub_id);
    this.wantedReactiveSpecs.delete(sub_id);
    if (!this.ctx.isLeader()) {
      this.ctx.broadcastToTabs({
        type: "sub-unregister",
        kind: "reactive",
        key: sub_id,
        sub_id,
      });
      return;
    }
    const owners = this.reactiveSubOwners.get(sub_id);
    if (owners) {
      owners.delete(OWN_TAB);
      if (owners.size === 0) {
        this.reactiveSubOwners.delete(sub_id);
        if (this.serverSubs.has(sub_id)) {
          this.serverSubs.unregister(sub_id, {
            type: "reactive-unsubscribe",
            sub_id,
          });
        }
      }
    }
  }

  // ---- Inbound from the multi-tab broker ---------------------------------

  /** Follower → leader: register a WS subscription on the follower's
   *  behalf. Caller (engine) has already gated on `isLeader()`. */
  handleForwardedRegister(
    msg: Record<string, unknown>,
    fromTabId: string,
  ): void {
    const kind = msg.kind as string;
    if (kind === "crdt") {
      const key = msg.key as string;
      const entity = msg.entity as string;
      const rowId = msg.rowId as string;
      // Track the forwarding follower in a separate set (NOT the
      // local refcount) — repeated sub-register from the same
      // follower stays idempotent, and a follower crash before
      // sub-unregister is cleaned up by `scrubPeer` when the
      // broker's `onLeave` fires.
      let fwd = this.crdtForwarders.get(key);
      if (!fwd) {
        fwd = new Set();
        this.crdtForwarders.set(key, fwd);
      }
      fwd.add(fromTabId);
      // Register if nothing else owned this key — local count and
      // forwarder set together gate the WS subscribe.
      const localCount = this.crdtSubscribers.get(key) ?? 0;
      if (localCount === 0 && fwd.size === 1) {
        this.serverSubs.register(key, {
          type: "crdt-subscribe",
          entity,
          rowId,
        });
      }
      return;
    }
    if (kind === "reactive") {
      const sub_id = msg.sub_id as string;
      const fn_name = msg.fn_name as string;
      const args = msg.args;
      let owners = this.reactiveSubOwners.get(sub_id);
      if (!owners) {
        owners = new Set();
        this.reactiveSubOwners.set(sub_id, owners);
      }
      owners.add(fromTabId);
      this.serverSubs.register(sub_id, {
        type: "reactive-subscribe",
        sub_id,
        fn_name,
        args,
      });
    }
  }

  /** Follower → leader: unregister a WS subscription on the follower's
   *  behalf. */
  handleForwardedUnregister(
    msg: Record<string, unknown>,
    fromTabId: string,
  ): void {
    const kind = msg.kind as string;
    if (kind === "crdt") {
      const key = msg.key as string;
      const entity = msg.entity as string;
      const rowId = msg.rowId as string;
      const fwd = this.crdtForwarders.get(key);
      if (fwd) {
        fwd.delete(fromTabId);
        if (fwd.size === 0) this.crdtForwarders.delete(key);
      }
      const localCount = this.crdtSubscribers.get(key) ?? 0;
      const remainingFwd = this.crdtForwarders.get(key)?.size ?? 0;
      if (localCount === 0 && remainingFwd === 0 && this.serverSubs.has(key)) {
        this.serverSubs.unregister(key, {
          type: "crdt-unsubscribe",
          entity,
          rowId,
        });
      }
      return;
    }
    if (kind === "reactive") {
      const sub_id = msg.sub_id as string;
      const owners = this.reactiveSubOwners.get(sub_id);
      if (!owners) return;
      owners.delete(fromTabId);
      if (owners.size === 0) {
        this.reactiveSubOwners.delete(sub_id);
        if (this.serverSubs.has(sub_id)) {
          this.serverSubs.unregister(sub_id, {
            type: "reactive-unsubscribe",
            sub_id,
          });
        }
      }
    }
  }

  /** Leader → follower: a `reactive-result` / `reactive-error` envelope
   *  arrived on the leader's WS. Route to the local handler if we
   *  registered the sub. */
  handleReactiveMessage(sub_id: string, payload: ReactiveMessage): void {
    const handler = this.reactiveHandlers.get(sub_id);
    if (handler) handler(payload);
  }

  // ---- Leader / follower transitions -------------------------------------

  /** Seed serverSubs with every subscription this tab currently wants.
   *  Called when this tab settles as leader (either at start() or via
   *  late promotion) so the next WS connect replays the bundle.
   *  Idempotent: serverSubs.register dedupes by payload equality. */
  seedFromLocalInterest(): void {
    for (const [sub_id, spec] of this.wantedReactiveSpecs) {
      this.serverSubs.register(sub_id, {
        type: "reactive-subscribe",
        sub_id,
        fn_name: spec.fn_name,
        args: spec.args,
      });
      let owners = this.reactiveSubOwners.get(sub_id);
      if (!owners) {
        owners = new Set();
        this.reactiveSubOwners.set(sub_id, owners);
      }
      owners.add(OWN_TAB);
    }
    for (const [key] of this.crdtSubscribers) {
      const [entity, rowId] = key.split("\x00");
      if (entity && rowId !== undefined) {
        this.serverSubs.register(key, {
          type: "crdt-subscribe",
          entity,
          rowId,
        });
      }
    }
  }

  /** Re-broadcast every locally-wanted sub to the (new) leader.
   *  Triggered by `request-sub-replay` after a leader change while we
   *  stayed a follower. */
  replayForwardedSubs(): void {
    for (const [sub_id, spec] of this.wantedReactiveSpecs) {
      this.ctx.broadcastToTabs({
        type: "sub-register",
        kind: "reactive",
        key: sub_id,
        sub_id,
        fn_name: spec.fn_name,
        args: spec.args,
      });
    }
    for (const [key] of this.crdtSubscribers) {
      const [entity, rowId] = key.split("\x00");
      if (entity && rowId !== undefined) {
        this.ctx.broadcastToTabs({
          type: "sub-register",
          kind: "crdt",
          key,
          entity,
          rowId,
        });
      }
    }
  }

  /** A peer tab disappeared (broker `bye` fired). Drop it from every
   *  forwarder/owner set; if a key drops to zero remaining owners
   *  (no local consumer, no forwarder) we unregister the WS sub so
   *  the server stops fanning at a tab that no longer exists. */
  scrubPeer(tabId: string): void {
    for (const [key, fwd] of this.crdtForwarders) {
      if (!fwd.delete(tabId)) continue;
      if (fwd.size === 0) {
        this.crdtForwarders.delete(key);
        const localCount = this.crdtSubscribers.get(key) ?? 0;
        if (localCount === 0 && this.serverSubs.has(key)) {
          const [entity, rowId] = key.split("\x00");
          if (entity && rowId !== undefined) {
            this.serverSubs.unregister(key, {
              type: "crdt-unsubscribe",
              entity,
              rowId,
            });
          }
        }
      }
    }
    for (const [sub_id, owners] of this.reactiveSubOwners) {
      if (!owners.delete(tabId)) continue;
      if (owners.size === 0) {
        this.reactiveSubOwners.delete(sub_id);
        if (this.serverSubs.has(sub_id)) {
          this.serverSubs.unregister(sub_id, {
            type: "reactive-unsubscribe",
            sub_id,
          });
        }
      }
    }
  }

  // ---- Predicates used by the engine -------------------------------------

  /** True when at least one follower tab has forwarded a CRDT sub.
   *  The engine gates its binary-broadcast on this so we don't fan
   *  binary frames to every tab when no follower cares about CRDT. */
  hasCrdtForwarders(): boolean {
    return this.crdtForwarders.size > 0;
  }
}

/** Internal CRDT key format: `${entity}\x00${rowId}`. Centralized so
 *  the engine's binary-frame parser (and any future routing logic)
 *  agrees with the coordinator on the wire shape. */
export function crdtKey(entity: string, rowId: string): string {
  return `${entity}\x00${rowId}`;
}
