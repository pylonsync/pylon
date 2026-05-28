// MultiTabOrchestrator — owns the cross-tab coordination protocol.
//
// Responsibilities:
//   - Bring up the BroadcastChannel-backed broker and run the
//     election protocol via `MultiTabBroker`.
//   - Route inbound BroadcastChannel messages through the right hook
//     on the engine OR directly through the SubscriptionCoordinator
//     when the case is purely a subscription concern.
//   - Provide typed broadcast helpers for outbound fanout (applied
//     changes, session updates, reset, mutations, reactive results,
//     binary frames, sub-replay requests).
//
// Why it lives outside the SyncEngine: the engine used to own the
// broker field, the isLeader flag, the 11-case `handleMultiTabMessage`
// switch, and every `broadcastToTabs` call. That bundle is its own
// concern — the cross-tab protocol — and pulling it out lets the
// engine focus on data-plane work (pull / push / reconcile / apply /
// subscribe). The engine still owns the data semantics behind each
// message; the orchestrator just decides "who sees what and when."
//
// Leader/follower bookkeeping: the orchestrator owns `isLeader` and
// notifies the engine via hooks when it flips. The engine keeps a
// mirror flag (`isMultiTabLeader`) so the existing call sites that
// gate on leadership don't need to round-trip into the orchestrator
// for every check.

import { MultiTabBroker } from "./multi-tab";
import type { PendingMutation } from "./mutation-queue";
import type { SubscriptionCoordinator } from "./subscription-coordinator";
import type {
  ChangeEvent,
  ReactiveMessage,
  ResolvedSession,
  Row,
  SyncCursor,
} from "./types";

/** ms the orchestrator waits for the broker's first election to
 *  settle before forcing the leader role if no peer claimed it. */
const ELECTION_SETTLE_MAX_MS = 400;

/** Hooks the engine registers to receive inbound multi-tab events.
 *  Cases that purely concern subscriptions are NOT routed through
 *  these — the orchestrator dispatches them directly to its
 *  `SubscriptionCoordinator` reference. */
export interface MultiTabOrchestratorHooks {
  /** Initial promotion (first election settles with this tab as
   *  leader). Fired before the engine's start() proceeds past
   *  `initMultiTab()`. */
  onInitialLeader(): void;
  /** Late promotion: the previous leader dropped while we were a
   *  follower. Engine performs recovery (replay subs, pull, drain
   *  the mutation queue, start the transport). */
  onLatePromote(): void;
  /** Demotion: another tab took over as leader. Engine tears down
   *  its transport. */
  onDemote(): void;
  /** Inbound applied-change batch from the current leader. Engine
   *  enqueues for apply with `fromBroadcast: true` so the queue
   *  doesn't re-broadcast. */
  onAppliedReceived(
    changes: ChangeEvent[],
    targetCursor: SyncCursor | undefined,
  ): void;
  /** Inbound reconcile batch from the current leader. */
  onReconciledReceived(
    entity: string,
    upserts: Row[],
    removalIds: string[],
    tombstoneSeq: number,
  ): void;
  /** Replica reset broadcast — identity flip happened on the leader. */
  onResetReceived(): void;
  /** Resolved session update from the leader. Engine funnels through
   *  its session chain so concurrent triggers commit in order. */
  onSessionReceived(resolved: ResolvedSession): void;
  /** Follower → leader: ops the follower wants pushed. Only fires
   *  when this tab is leader. */
  onMutationsForwarded(ops: PendingMutation[]): void;
  /** Leader → follower: op_ids that were successfully pushed. */
  onMutationsAcked(opIds: string[]): void;
  /** Leader → follower: op_ids that failed server-side validation. */
  onMutationsFailed(ops: { opId: string; error: string }[]): void;
  /** Leader → follower: a binary frame from the WS. Engine routes
   *  to its local binary handlers. */
  onBinaryReceived(bytes: Uint8Array): void;
  /** A peer tab disappeared (broker observed `bye`). Engine and
   *  SubscriptionCoordinator both clean up state for the departed tab. */
  onPeerLeft(tabId: string): void;
}

export interface MultiTabOrchestratorConfig {
  /** When false, multi-tab coordination is disabled; this tab acts
   *  as a sole leader. */
  enabled?: boolean;
  /** App name used to derive the BroadcastChannel name. Defaults to
   *  "default" — apps that share an origin can pin their own name
   *  to avoid cross-talk between unrelated installations. */
  appName?: string;
}

export class MultiTabOrchestrator {
  private broker: MultiTabBroker | null = null;
  private _isLeader = false;
  private settled = false;

  constructor(
    private readonly config: MultiTabOrchestratorConfig,
    private readonly subscriptions: SubscriptionCoordinator,
    private readonly hooks: MultiTabOrchestratorHooks,
  ) {}

  // ---- Lifecycle ---------------------------------------------------------

  /** Bring up the broker and run the initial election. Resolves when
   *  the election has settled (either onPromote fired, or the
   *  settle timer expired without a peer claiming leadership).
   *  Returns true if this tab is now the leader. */
  async init(): Promise<boolean> {
    if (this.config.enabled === false) {
      this._isLeader = true;
      this.settled = true;
      this.hooks.onInitialLeader();
      return true;
    }
    if (!MultiTabBroker.available()) {
      this._isLeader = true;
      this.settled = true;
      this.hooks.onInitialLeader();
      return true;
    }
    const channelName = `pylon:${this.config.appName ?? "default"}:multitab`;
    this.broker = new MultiTabBroker();
    await new Promise<void>((resolve) => {
      const finish = () => {
        if (this.settled) return;
        this.settled = true;
        resolve();
      };
      this.broker!.start(channelName, {
        onPromote: () => {
          this._isLeader = true;
          if (!this.settled) {
            // Initial promotion fired before the settle timer — the
            // engine's start() proceeds straight into the leader path.
            this.hooks.onInitialLeader();
            finish();
          } else {
            // Late promotion: the previous leader dropped after our
            // initial settle. Engine recovers (pull, replay subs,
            // drain queued mutations, start transport).
            this.hooks.onLatePromote();
          }
        },
        onDemote: () => {
          this._isLeader = false;
          this.hooks.onDemote();
        },
        onAppMessage: (payload, from) =>
          this.handleMessage(payload, from.tabId),
        onLeave: (tabId) => {
          // Drop forwarded-sub state for the departed tab AND notify
          // the engine in case it wants to do anything else (the
          // engine's current hook just delegates to subscriptions,
          // but the seam is here for future cleanup needs).
          if (this._isLeader) this.subscriptions.scrubPeer(tabId);
          this.hooks.onPeerLeft(tabId);
        },
      });
      // Bound the settle wait: if no peer claims leader within the
      // election window, this tab takes the role.
      setTimeout(() => {
        if (this.settled) return;
        if (this.broker!.isLeader()) {
          this._isLeader = true;
          this.hooks.onInitialLeader();
        }
        finish();
      }, ELECTION_SETTLE_MAX_MS);
    });
    return this._isLeader;
  }

  stop(): void {
    if (this.broker) {
      this.broker.stop();
      this.broker = null;
    }
  }

  // ---- Predicates --------------------------------------------------------

  isLeader(): boolean {
    return this._isLeader;
  }

  // ---- Outbound broadcasts ----------------------------------------------

  /** No-op when the broker isn't running. The engine uses this for
   *  envelope shapes the orchestrator doesn't have first-class
   *  helpers for (currently none — kept as an escape hatch). */
  broadcastRaw(payload: unknown): void {
    this.broker?.broadcastApp(payload);
  }

  broadcastApplied(changes: ChangeEvent[], targetCursor?: SyncCursor): void {
    this.broadcastRaw({ type: "applied", changes, targetCursor });
  }

  broadcastReconciled(
    entity: string,
    upserts: Row[],
    removalIds: string[],
    tombstoneSeq: number,
  ): void {
    this.broadcastRaw({
      type: "reconciled",
      entity,
      upserts,
      removalIds,
      tombstoneSeq,
    });
  }

  broadcastReset(): void {
    this.broadcastRaw({ type: "reset" });
  }

  broadcastSession(resolved: ResolvedSession): void {
    this.broadcastRaw({ type: "session", resolved });
  }

  /** Follower → leader: forward our pending batch. The leader's
   *  engine handles it via `onMutationsForwarded`. */
  forwardMutations(ops: PendingMutation[]): void {
    this.broadcastRaw({ type: "mutations", ops });
  }

  /** Leader → followers: applied op_ids. Followers mark applied + clear. */
  broadcastMutationsAcked(opIds: string[]): void {
    this.broadcastRaw({ type: "mutations-acked", opIds });
  }

  /** Leader → followers: per-op failures with error strings. */
  broadcastMutationsFailed(ops: { opId: string; error: string }[]): void {
    this.broadcastRaw({ type: "mutations-failed", ops });
  }

  /** Leader → followers: a reactive-result / reactive-error landed on
   *  the WS. Routed by sub_id to whatever tab owns the local handler. */
  broadcastReactiveMessage(sub_id: string, payload: ReactiveMessage): void {
    this.broadcastRaw({ type: "reactive-msg", sub_id, payload });
  }

  /** Leader → followers: a binary CRDT frame from the WS. Only
   *  meaningful when at least one follower has forwarded a CRDT
   *  sub — caller (engine) gates on `subscriptions.hasCrdtForwarders()`. */
  broadcastBinary(bytes: Uint8Array): void {
    this.broadcastRaw({ type: "binary", bytes });
  }

  /** New leader → followers: re-forward your active sub-registers. */
  requestSubReplay(): void {
    this.broadcastRaw({ type: "request-sub-replay" });
  }

  // ---- Inbound dispatch --------------------------------------------------

  /** Public for tests + future debugging. Drives the same path the
   *  BroadcastChannel's onmessage hits, so a test can simulate any
   *  inbound envelope without standing up a real channel. The engine
   *  itself never calls this directly — it goes through the broker. */
  handleMessage(payload: unknown, fromTabId: string): void {
    if (!payload || typeof payload !== "object") return;
    const msg = payload as { type?: string } & Record<string, unknown>;
    switch (msg.type) {
      case "applied": {
        const changes = msg.changes as ChangeEvent[] | undefined;
        const targetCursor = msg.targetCursor as SyncCursor | undefined;
        if (Array.isArray(changes) || targetCursor) {
          this.hooks.onAppliedReceived(changes ?? [], targetCursor);
        }
        break;
      }
      case "reconciled": {
        const entity = msg.entity as string | undefined;
        const upserts = msg.upserts as Row[] | undefined;
        const removalIds = msg.removalIds as string[] | undefined;
        const tombstoneSeq = msg.tombstoneSeq as number | undefined;
        if (
          typeof entity === "string" &&
          Array.isArray(upserts) &&
          Array.isArray(removalIds) &&
          typeof tombstoneSeq === "number"
        ) {
          this.hooks.onReconciledReceived(
            entity,
            upserts,
            removalIds,
            tombstoneSeq,
          );
        }
        break;
      }
      case "reset": {
        this.hooks.onResetReceived();
        break;
      }
      case "session": {
        const resolved = msg.resolved as ResolvedSession | undefined;
        if (resolved) this.hooks.onSessionReceived(resolved);
        break;
      }
      case "mutations": {
        // Follower → leader. Only the leader acts on these.
        if (!this._isLeader) return;
        const ops = msg.ops as PendingMutation[] | undefined;
        if (Array.isArray(ops)) this.hooks.onMutationsForwarded(ops);
        break;
      }
      case "mutations-acked": {
        const opIds = msg.opIds as string[] | undefined;
        if (Array.isArray(opIds)) this.hooks.onMutationsAcked(opIds);
        break;
      }
      case "mutations-failed": {
        const ops = msg.ops as
          | { opId: string; error: string }[]
          | undefined;
        if (Array.isArray(ops)) this.hooks.onMutationsFailed(ops);
        break;
      }
      case "sub-register": {
        // Follower → leader. Leader-only — no work to do on a follower
        // because it can't act on a peer's request.
        if (!this._isLeader) return;
        this.subscriptions.handleForwardedRegister(msg, fromTabId);
        break;
      }
      case "sub-unregister": {
        if (!this._isLeader) return;
        this.subscriptions.handleForwardedUnregister(msg, fromTabId);
        break;
      }
      case "reactive-msg": {
        const sub_id = msg.sub_id as string;
        const payload = msg.payload as ReactiveMessage;
        this.subscriptions.handleReactiveMessage(sub_id, payload);
        break;
      }
      case "binary": {
        const bytes = msg.bytes as Uint8Array | undefined;
        if (bytes instanceof Uint8Array) this.hooks.onBinaryReceived(bytes);
        break;
      }
      case "request-sub-replay": {
        // Followers respond. A leader receiving its own broadcast (or
        // an old broadcast post-promotion) ignores — its serverSubs
        // already has the bundle.
        if (this._isLeader) return;
        this.subscriptions.replayForwardedSubs();
        break;
      }
    }
  }
}
