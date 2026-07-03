// RoomSubscriptions — client-side registry for WS-pushed room membership.
//
// The server (>=v0.3.214) accepts `{ type: "room-subscribe", room }` and
// `{ type: "room-unsubscribe", room }` control frames, and pushes back
// `room-snapshot` (members list) + `room-update` (join/leave/presence/
// broadcast deltas). This registry owns the client-side bookkeeping:
//
//   - refcount per roomId so N `useRoom` mounts of the same channel
//     produce exactly one `room-subscribe` on the wire (the first add)
//     and one `room-unsubscribe` (the last remove). Mirrors the dedup
//     story `useRoom`'s `__roomRegistryInternals` already enforced at
//     the React layer — but moved INTO the engine because the WS sub
//     is the engine's connection, not React's.
//
//   - per-room subscriber callbacks. Server pushes are O(rooms), but
//     each push has to fan out to every component that subscribed to
//     that room. We notify them all when the snapshot or update lands.
//
//   - cached `members` snapshot so a late subscriber (mounting after
//     the initial snapshot already landed) gets the current state on
//     `register()` without waiting for the next push.
//
//   - replay-on-reconnect. Server clears its per-client room
//     subscription state on disconnect; on reconnect, we resend a
//     `room-subscribe` for every roomId that still has subscribers.
//
// Why a separate registry from ServerSubscriptions:
//   ServerSubscriptions is a write-only replay registry — it remembers
//   the SUBSCRIBE message and re-sends it on reconnect. Rooms need
//   more than that: per-room state (the members snapshot), per-room
//   callbacks, and per-room error surfacing. Mixing those concerns
//   into ServerSubscriptions would dilute its single responsibility.
//   Both are used together by the engine: ServerSubscriptions handles
//   the replay primitive; RoomSubscriptions is the consumer that
//   keys into it for rooms.
//
// Multi-tab note: only the leader tab opens a WS, so only the leader
// runs this registry against a real socket. Follower tabs broadcast
// `room-sub-register` envelopes to the leader, the leader keeps a
// per-room follower set so the WS sub stays alive while any tab in
// the origin still wants it, and inbound `room-snapshot`/`room-update`
// land on the leader's WS and get fanned out cross-tab so each
// follower's local registry routes to its own React subscribers.
// The leader-side fanout / follower-side mirror plumbing lives on
// the engine + orchestrator; this module is leader-local state.

export interface RoomMember {
  user_id: string;
  joined_at: string;
  data?: Record<string, unknown>;
}

/** Reason codes the SDK exposes on a room subscription error. */
export type RoomErrorCode = "NOT_IN_ROOM" | "UNKNOWN";

export interface RoomError {
  code: RoomErrorCode;
  message?: string;
}

/** Callback fired when a room's membership snapshot OR error state
 *  changes. The same callback receives every transition for the room —
 *  snapshots overwrite state, updates mutate it, and errors set the
 *  `error` slot. Subscribers read the latest values via the getters on
 *  the entry; this fires purely as a "something changed" pulse so
 *  React hooks can re-render. */
export type RoomSubscriber = () => void;

/** A broadcast message relayed through a room. `from` is the sender's
 *  user id (the server stamps it — clients can't spoof each other). */
export interface RoomMessage {
  topic: string;
  payload: unknown;
  from: string;
}

export type RoomMessageSubscriber = (message: RoomMessage) => void;

interface RoomEntry {
  roomId: string;
  /** Current members snapshot (post-snapshot/update). `null` until the
   *  first snapshot lands — distinct from "empty room" which is `[]`.
   *  React hooks distinguish loading vs empty using this. */
  members: RoomMember[] | null;
  /** Latest error from the server (NOT_IN_ROOM). Cleared when a fresh
   *  snapshot lands. */
  error: RoomError | null;
  /** Number of `register()` calls that haven't been balanced by
   *  `unregister()`. The first add ships `room-subscribe`; the last
   *  remove ships `room-unsubscribe`. Message subscribers count too —
   *  a tab that only listens for broadcasts still needs the wire sub. */
  refs: number;
  /** Subscriber callbacks — one per mounted React hook. */
  subs: Set<RoomSubscriber>;
  /** Broadcast-message callbacks. Unlike `subs` (membership pulses),
   *  these receive the actual relayed payloads. */
  messageSubs: Set<RoomMessageSubscriber>;
}

export class RoomSubscriptions {
  private readonly rooms: Map<string, RoomEntry> = new Map();

  /** Caller-supplied uplink to the WS. The engine routes through its
   *  active transport; this module stays transport-agnostic.
   *  Returns true when the message reached the wire (transport is open),
   *  false otherwise — RoomSubscriptions uses this to decide whether
   *  to surface "not connected yet" to the registry's callers.
   *  No-op transports (followers, no WS yet) return false; the engine
   *  hides the broadcast/leader split behind this hook. */
  constructor(private readonly sendWs: (msg: unknown) => boolean) {}

  /** Register a subscriber for `roomId`. First add ships the WS
   *  `room-subscribe`; subsequent adds just bump the refcount and
   *  deliver the cached snapshot to the new subscriber's callback.
   *
   *  Returns an unsubscribe function that decrements the refcount on
   *  call. The last unsubscribe ships `room-unsubscribe`, clears the
   *  entry, and the next register() for the same room is a fresh
   *  start.
   *
   *  Idempotent w.r.t. wire frames: a re-subscribe with no intervening
   *  full unsubscribe doesn't re-send `room-subscribe` to the server. */
  register(roomId: string, subscriber: RoomSubscriber): () => void {
    const { entry, isFirst } = this.ensureEntry(roomId);
    entry.refs += 1;
    entry.subs.add(subscriber);

    if (isFirst) {
      // First subscriber — ask the server to start pushing updates.
      // `sendWs` returns false when the transport isn't open yet (or
      // we're a follower); that's fine — `replay()` on reconnect /
      // promotion will re-send the subscribe.
      this.sendWs({ type: "room-subscribe", room: roomId });
    } else if (entry.members !== null || entry.error !== null) {
      // Late subscriber on an already-active room. Fire one tick so
      // the new callback observes the cached snapshot / error
      // immediately instead of waiting for the next push.
      try {
        subscriber();
      } catch (err) {
        console.warn("[sync] room subscriber threw on initial notify:", err);
      }
    }

    return () => this.unregisterOne(roomId, subscriber);
  }

  /** Get-or-create the entry for a room. `isFirst` means this call
   *  created it, i.e. the caller must ship the wire `room-subscribe`. */
  private ensureEntry(roomId: string): { entry: RoomEntry; isFirst: boolean } {
    let entry = this.rooms.get(roomId);
    const isFirst = !entry;
    if (!entry) {
      entry = {
        roomId,
        members: null,
        error: null,
        refs: 0,
        subs: new Set(),
        messageSubs: new Set(),
      };
      this.rooms.set(roomId, entry);
    }
    return { entry, isFirst };
  }

  /**
   * Register a BROADCAST-MESSAGE listener for `roomId`. Counts toward
   * the same refcount as membership subscribers (a tab that only
   * listens for broadcasts still needs the wire `room-subscribe`).
   * The callback receives every `action: "broadcast"` relay for the
   * room — including the caller's own broadcasts echoed back; filter
   * on `message.from` if self-echo is unwanted.
   */
  registerMessages(roomId: string, subscriber: RoomMessageSubscriber): () => void {
    const { entry, isFirst } = this.ensureEntry(roomId);
    entry.refs += 1;
    entry.messageSubs.add(subscriber);
    if (isFirst) {
      this.sendWs({ type: "room-subscribe", room: roomId });
    }
    return () => {
      const e = this.rooms.get(roomId);
      if (!e || !e.messageSubs.delete(subscriber)) return;
      e.refs -= 1;
      if (e.refs > 0) return;
      this.rooms.delete(roomId);
      this.sendWs({ type: "room-unsubscribe", room: roomId });
    };
  }

  /** Decrement the refcount for one subscriber. Internal — the
   *  `register()` returned unsubscribe routes here. Last out ships
   *  `room-unsubscribe`. */
  private unregisterOne(roomId: string, subscriber: RoomSubscriber): void {
    const entry = this.rooms.get(roomId);
    if (!entry) return;
    if (!entry.subs.delete(subscriber)) return;
    entry.refs -= 1;
    if (entry.refs > 0) return;
    this.rooms.delete(roomId);
    this.sendWs({ type: "room-unsubscribe", room: roomId });
  }

  /** Force a full teardown of one room regardless of refcount. Used by
   *  the engine's leader-handoff and the hook's manual `leave()` path.
   *  Notifies every remaining subscriber via the standard pulse (so
   *  they observe `members === null` and re-render as disconnected). */
  unregisterRoom(roomId: string): void {
    const entry = this.rooms.get(roomId);
    if (!entry) return;
    this.rooms.delete(roomId);
    this.sendWs({ type: "room-unsubscribe", room: roomId });
  }

  /** Snapshot push from the server: full membership for the room.
   *  Overwrites any cached state, clears any prior error, and pulses
   *  every subscriber callback. */
  applySnapshot(roomId: string, members: RoomMember[]): void {
    const entry = this.rooms.get(roomId);
    if (!entry) return;
    entry.members = members;
    entry.error = null;
    this.notify(entry);
  }

  /** Incremental update from the server. `action` mirrors the server's
   *  RoomEvent variants — join / leave / presence / broadcast.
   *  Membership actions mutate the cached `members` snapshot in-place
   *  and pulse the membership subscribers; `broadcast` actions route
   *  the relayed payload to the message subscribers instead (and do
   *  NOT pulse membership — fire-rate broadcasts would otherwise
   *  re-render every useRoom consumer per message). */
  applyUpdate(
    roomId: string,
    action: "join" | "leave" | "presence" | "broadcast",
    member: RoomMember | undefined,
    _data: unknown,
  ): void {
    const entry = this.rooms.get(roomId);
    if (!entry) return;
    // If we haven't received a snapshot yet, seed an empty list so
    // the in-place mutation paths below have something to mutate.
    if (entry.members === null) entry.members = [];
    switch (action) {
      case "join": {
        if (!member) break;
        // Idempotent insert: server may re-send a join on flapping
        // connections; collapse by user_id.
        const filtered = entry.members.filter(
          (m) => m.user_id !== member.user_id,
        );
        filtered.push(member);
        entry.members = filtered;
        break;
      }
      case "leave": {
        if (!member) break;
        entry.members = entry.members.filter(
          (m) => m.user_id !== member.user_id,
        );
        break;
      }
      case "presence": {
        if (!member) break;
        // Swap the matching member's data while preserving join order.
        // The server ships the NEW presence in the envelope's data slot
        // (`member` carries only the user_id) — merging `member` alone
        // is a no-op that drops every live presence update: remote
        // cursors/typing indicators freeze at whatever the join carried.
        const presence =
          _data && typeof _data === "object"
            ? (_data as Record<string, unknown>)
            : undefined;
        entry.members = entry.members.map((m) =>
          m.user_id === member.user_id
            ? { ...m, ...member, ...(presence ? { data: presence } : {}) }
            : m,
        );
        break;
      }
      case "broadcast": {
        // No membership delta — route the payload to the message
        // listeners and return WITHOUT pulsing membership subscribers
        // (game fire events broadcast at ~10 Hz; pulsing would
        // re-render every useRoom consumer per message).
        const raw = (_data ?? {}) as { topic?: unknown; payload?: unknown };
        const message: RoomMessage = {
          topic: typeof raw.topic === "string" ? raw.topic : "",
          payload: raw.payload,
          from: member?.user_id ?? "",
        };
        for (const cb of entry.messageSubs) {
          try {
            cb(message);
          } catch (err) {
            console.warn("[sync] room message subscriber threw:", err);
          }
        }
        return;
      }
    }
    entry.error = null;
    this.notify(entry);
  }

  /** Server pushed `{ type: "error", code, room }` after a subscribe.
   *  Record the error on the room and pulse subscribers — the React
   *  hook surfaces it via the `error` return slot. We do NOT
   *  unregister automatically: the user genuinely isn't in the room
   *  and the SDK shouldn't retry, but the registry entry stays so
   *  React unmount still ships a `room-unsubscribe` (server is
   *  idempotent for unknown subs). */
  applyError(roomId: string, error: RoomError): void {
    const entry = this.rooms.get(roomId);
    if (!entry) return;
    entry.error = error;
    this.notify(entry);
  }

  /** Read the cached snapshot for a room without subscribing. The
   *  React hook uses this in its initial-state effect so a re-mount
   *  inside the same registry lifecycle picks up the current members
   *  on tick zero. Returns `null` when the snapshot hasn't landed
   *  yet — DISTINCT from `[]` ("empty room"). */
  members(roomId: string): RoomMember[] | null {
    return this.rooms.get(roomId)?.members ?? null;
  }

  /** Latest error for a room (null if none). */
  error(roomId: string): RoomError | null {
    return this.rooms.get(roomId)?.error ?? null;
  }

  /** Is there at least one local subscriber for `roomId`. Used by the
   *  hook's polling-fallback path to decide whether to keep polling. */
  has(roomId: string): boolean {
    return this.rooms.has(roomId);
  }

  /** Every room currently tracked. Used by `replay()` and by the
   *  multi-tab seed-on-promotion. */
  roomIds(): string[] {
    return Array.from(this.rooms.keys());
  }

  /** Resend `room-subscribe` for every active room. Called by the
   *  engine's `onConnected` hook after the WS reopens — the server
   *  forgets per-client subs across disconnects, so without this
   *  resync the first push would never arrive. */
  replay(): void {
    for (const roomId of this.rooms.keys()) {
      this.sendWs({ type: "room-subscribe", room: roomId });
    }
  }

  /** Test/diagnostics: total active rooms. */
  size(): number {
    return this.rooms.size;
  }

  private notify(entry: RoomEntry): void {
    for (const cb of entry.subs) {
      try {
        cb();
      } catch (err) {
        console.warn("[sync] room subscriber threw:", err);
      }
    }
  }
}
