// MultiTabBroker — leader-elect coordination between SyncEngine
// instances in different browser tabs of the same origin.
//
// Without this, every tab opens its own WS, runs its own pull,
// pushes its own copy of each hydrated offline mutation, and writes
// duplicates to IndexedDB. The server dedupes mutations by op_id and
// fanout fires WS events to every connected tab — so it isn't
// strictly broken, just wasteful (N tabs = N sockets, N pulls). And
// optimistic mutations made in tab A aren't visible in tab B until
// the server's WS broadcast lands, which is a real coherence gap.
//
// Design: one tab is the leader. The leader owns the WS connection,
// drives pull/push/reconcile, and broadcasts every applied change
// over a BroadcastChannel. Followers mirror the changes locally,
// forward their mutations to the leader for pushing, and forward
// their subscribe/unsubscribe to the leader for WS registration.
//
// Election protocol:
// - Each tab generates a (startTime, tabId) on construction.
// - On `start`, every tab broadcasts a `hello`. Tabs respond with
//   `here` listing their own (startTime, tabId).
// - After a 250ms settle window, each tab knows every other tab's
//   identity. The smallest (startTime, tabId) is the leader.
// - The leader broadcasts `lead` every LEADER_HEARTBEAT_MS so
//   followers know it's still alive.
// - If a follower doesn't see a `lead` for LEADER_TIMEOUT_MS, it
//   declares the leader dead, drops it from the roster, and re-elects.
// - Election is purely deterministic (smallest tuple) — no voting,
//   no split-brain. Two tabs can't both think they're leader unless
//   one is on a clock that drifted by minutes, in which case op_id
//   dedupe at the server still catches the dup writes.
//
// All messages flow through a single BroadcastChannel scoped by
// appName so two apps on the same origin don't cross-talk. The
// broker is silent (no-op leader = self) when BroadcastChannel is
// unavailable (Node, jsdom without polyfill, very old Safari).

import { generateId } from "./ids";

/** ms between heartbeats from the leader. */
const LEADER_HEARTBEAT_MS = 1_000;
/** ms a follower waits without a heartbeat before re-electing. */
const LEADER_TIMEOUT_MS = 3_000;
/** ms a tab waits after `hello` to collect responses before deciding. */
const ELECTION_SETTLE_MS = 250;

interface TabIdentity {
  tabId: string;
  startTime: number;
}

type BrokerMessage =
  | { type: "hello"; tab: TabIdentity }
  | {
      type: "here";
      tab: TabIdentity;
      leaderTabId: string | null;
      /** Full known roster, including self. Late joiners pick up
       *  every tab they need to consider in the election from a
       *  single `here` rather than waiting for N separate replies. */
      roster: TabIdentity[];
    }
  | { type: "lead"; tab: TabIdentity }
  | { type: "bye"; tabId: string }
  | { type: "app"; tab: TabIdentity; payload: unknown };

export interface MultiTabHandlers {
  /** Fired when this tab becomes the leader (initial election or
   *  promotion after the previous leader dropped). Idempotent —
   *  called once per promotion. */
  onPromote: () => void;
  /** Fired when this tab transitions from leader to follower. Only
   *  relevant if leader migration is supported (currently no-op
   *  because the leader stays leader until it dies). */
  onDemote: () => void;
  /** Fired for every application-level message from another tab. */
  onAppMessage: (payload: unknown, from: TabIdentity) => void;
}

export class MultiTabBroker {
  readonly self: TabIdentity = {
    tabId: generateId(),
    startTime: Date.now(),
  };
  private channel: BroadcastChannel | null = null;
  /** Roster of currently-known tabs (including self). Maintained via
   *  `hello` / `here` / `bye`. The smallest entry by
   *  (startTime, tabId) is the leader. */
  private roster: Map<string, TabIdentity> = new Map();
  private leaderTabId: string | null = null;
  private lastLeaderHeartbeat = 0;
  private heartbeatTimer: ReturnType<typeof setInterval> | null = null;
  private monitorTimer: ReturnType<typeof setInterval> | null = null;
  private electionTimer: ReturnType<typeof setTimeout> | null = null;
  private handlers: MultiTabHandlers | null = null;
  private started = false;
  private stopped = false;

  /** Whether the broker can run on this platform — `BroadcastChannel`
   *  is missing in Node/jsdom and old Safari. When false the engine
   *  treats this tab as the implicit sole leader. */
  static available(): boolean {
    return typeof BroadcastChannel !== "undefined";
  }

  /** Begin participating in multi-tab coordination. The handlers
   *  fire as elections settle. Idempotent — calling twice is a no-op
   *  so the engine's `start()` can call freely. */
  start(channelName: string, handlers: MultiTabHandlers): void {
    if (this.started || this.stopped) return;
    this.started = true;
    this.handlers = handlers;
    if (!MultiTabBroker.available()) {
      // No BroadcastChannel — declare self leader and stop. The engine
      // runs normally; this just makes the no-multi-tab case identical
      // to "I'm the only tab".
      this.leaderTabId = this.self.tabId;
      this.roster.set(this.self.tabId, this.self);
      handlers.onPromote();
      return;
    }
    this.channel = new BroadcastChannel(channelName);
    this.channel.onmessage = (ev: MessageEvent) =>
      this.handle(ev.data as BrokerMessage);
    this.roster.set(this.self.tabId, this.self);
    // Announce; existing tabs reply with `here`, fresh tabs see this.
    this.send({ type: "hello", tab: this.self });
    // After the settle window, run a deterministic election against
    // whatever roster we collected.
    this.electionTimer = setTimeout(() => {
      this.electionTimer = null;
      this.electLeader();
    }, ELECTION_SETTLE_MS);
    // Monitor: catch a dead leader.
    this.monitorTimer = setInterval(
      () => this.checkLeaderLiveness(),
      LEADER_HEARTBEAT_MS,
    );
    // beforeunload: tell other tabs we're going so they can re-elect
    // immediately instead of waiting for the timeout.
    if (typeof window !== "undefined") {
      window.addEventListener("beforeunload", this.handleBeforeUnload);
    }
  }

  /** Tear down. Sends a `bye` so other tabs can re-elect right away. */
  stop(): void {
    if (this.stopped) return;
    this.stopped = true;
    this.started = false;
    if (this.channel) {
      try {
        this.send({ type: "bye", tabId: this.self.tabId });
      } catch {
        /* channel may already be closed */
      }
      this.channel.close();
      this.channel = null;
    }
    if (this.heartbeatTimer) clearInterval(this.heartbeatTimer);
    if (this.monitorTimer) clearInterval(this.monitorTimer);
    if (this.electionTimer) clearTimeout(this.electionTimer);
    this.heartbeatTimer = null;
    this.monitorTimer = null;
    this.electionTimer = null;
    if (typeof window !== "undefined") {
      window.removeEventListener("beforeunload", this.handleBeforeUnload);
    }
  }

  /** True if this tab is currently the leader. Follower paths use
   *  this to decide whether to run their own network ops. */
  isLeader(): boolean {
    return this.leaderTabId === this.self.tabId;
  }

  /** Send an application-level payload to every other tab. The
   *  engine uses this for applied-change / mutation / session
   *  broadcasts. */
  broadcastApp(payload: unknown): void {
    this.send({ type: "app", tab: this.self, payload });
  }

  // ---- internals ----------------------------------------------------------

  private handleBeforeUnload = (): void => {
    if (this.channel) {
      try {
        this.send({ type: "bye", tabId: this.self.tabId });
      } catch {
        /* best-effort */
      }
    }
  };

  private send(msg: BrokerMessage): void {
    this.channel?.postMessage(msg);
  }

  private handle(msg: BrokerMessage): void {
    if (!this.handlers) return;
    switch (msg.type) {
      case "hello": {
        // Someone new showed up. Add to roster and reply with `here`
        // carrying the full roster + current leader so the joiner can
        // converge from a single message rather than collecting N
        // separate replies.
        this.roster.set(msg.tab.tabId, msg.tab);
        this.send({
          type: "here",
          tab: this.self,
          leaderTabId: this.leaderTabId,
          roster: Array.from(this.roster.values()),
        });
        // The new tab may have a smaller (startTime, tabId) — re-elect
        // after a settle window so we converge on the same winner.
        this.scheduleElection();
        break;
      }
      case "here": {
        // Absorb the full roster so we elect against the same set as
        // the responder, not just self + responder.
        for (const t of msg.roster) {
          if (!this.roster.has(t.tabId)) this.roster.set(t.tabId, t);
        }
        this.roster.set(msg.tab.tabId, msg.tab);
        // Don't accept `here.leaderTabId` blindly — a clock-skewed
        // tab can claim leadership via its own `here` reply, and the
        // late joiner would treat it as canonical. Validate against
        // our (now larger) roster: only honor the claim if the
        // sender is the current computed winner.
        if (msg.leaderTabId) {
          const winner = this.computeWinner();
          if (winner && winner.tabId === msg.leaderTabId) {
            this.leaderTabId = msg.leaderTabId;
          }
        }
        break;
      }
      case "lead": {
        this.roster.set(msg.tab.tabId, msg.tab);
        // Don't trust a `lead` claim blindly. If the sender isn't the
        // minimum (startTime, tabId) in OUR known roster, ignore it —
        // a tab whose clock jumped backwards (macOS sleep/wake on a
        // VM) could otherwise convince its peers it's leader and
        // produce a ping-pong demote/promote loop. The next election
        // will reconcile. Crucially, do NOT update lastLeaderHeartbeat
        // for an ignored claim: otherwise the misbehaving tab would
        // suppress our liveness check and stay "leader" indefinitely.
        const winner = this.computeWinner();
        if (winner && winner.tabId !== msg.tab.tabId) {
          this.scheduleElection();
          break;
        }
        this.lastLeaderHeartbeat = Date.now();
        const wasLeader = this.isLeader();
        this.leaderTabId = msg.tab.tabId;
        if (wasLeader && !this.isLeader()) {
          this.demote();
        }
        break;
      }
      case "bye": {
        this.roster.delete(msg.tabId);
        if (this.leaderTabId === msg.tabId) {
          this.leaderTabId = null;
          this.scheduleElection();
        }
        break;
      }
      case "app": {
        // Don't echo our own broadcasts back to ourselves.
        if (msg.tab.tabId === this.self.tabId) return;
        this.handlers.onAppMessage(msg.payload, msg.tab);
        break;
      }
    }
  }

  private scheduleElection(): void {
    if (this.electionTimer) return;
    this.electionTimer = setTimeout(() => {
      this.electionTimer = null;
      this.electLeader();
    }, ELECTION_SETTLE_MS);
  }

  private electLeader(): void {
    const winner = this.computeWinner() ?? this.self;
    const wasLeader = this.isLeader();
    this.leaderTabId = winner.tabId;
    if (this.isLeader() && !wasLeader) {
      this.promote();
    } else if (!this.isLeader() && wasLeader) {
      this.demote();
    }
  }

  /** Smallest (startTime, tabId) wins. Deterministic across every
   *  tab that has the same roster, so peers converge without voting.
   *  Used both for the initial election and for sanity-checking
   *  `lead` claims against our local view. */
  private computeWinner(): TabIdentity | null {
    let winner: TabIdentity | null = null;
    for (const t of this.roster.values()) {
      if (!winner) {
        winner = t;
        continue;
      }
      if (
        t.startTime < winner.startTime ||
        (t.startTime === winner.startTime && t.tabId < winner.tabId)
      ) {
        winner = t;
      }
    }
    return winner;
  }

  private promote(): void {
    if (this.heartbeatTimer) return; // already promoted
    this.lastLeaderHeartbeat = Date.now();
    this.heartbeatTimer = setInterval(() => {
      this.send({ type: "lead", tab: this.self });
    }, LEADER_HEARTBEAT_MS);
    // Send one immediately so followers don't think we died during
    // the first heartbeat interval.
    this.send({ type: "lead", tab: this.self });
    this.handlers?.onPromote();
  }

  private demote(): void {
    if (this.heartbeatTimer) clearInterval(this.heartbeatTimer);
    this.heartbeatTimer = null;
    this.handlers?.onDemote();
  }

  private checkLeaderLiveness(): void {
    if (this.isLeader()) return;
    if (!this.leaderTabId) {
      // No known leader — trigger an election.
      this.scheduleElection();
      return;
    }
    const elapsed = Date.now() - this.lastLeaderHeartbeat;
    if (elapsed > LEADER_TIMEOUT_MS) {
      // Leader hasn't checked in. Drop from roster and re-elect.
      this.roster.delete(this.leaderTabId);
      this.leaderTabId = null;
      this.scheduleElection();
    }
  }
}
