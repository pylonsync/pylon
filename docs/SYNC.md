# Pylon: Sync Semantics

What Pylon's sync engine actually does, what it does not do, and where we
draw the line. Read this before claiming Pylon is "local-first" — that
phrase has a specific meaning and Pylon does not meet it.

## The honest one-liner

**Pylon is server-authoritative sync with optimistic mutations and an
offline write queue. It is not local-first in the Ink & Switch sense — it
does not use CRDTs, does not perform offline conflict merging, and the
server's serialization order is canonical.** Concurrent writers can lose
field values when both edit the same column.

If you need true local-first (multi-device convergence, no central
authority, offline-capable conflict resolution), Pylon's current sync
layer isn't the right tool — see *Roadmap* below.

## What you actually get

### 1. Server-assigned monotonic sequence numbers

Every change applied on the server gets a monotonically increasing `seq`.
Clients pull from a cursor (`since=<last_seq>`) and apply changes in
seq order. This gives every client a deterministic view of the server's
write history.

```rust
// crates/sync/src/lib.rs
pub fn append(&self, entity: &str, row_id: &str, kind: ChangeKind, data: Option<Value>) -> u64 {
    let mut seq = self.seq.lock().unwrap();
    *seq += 1;
    let event = ChangeEvent { seq: *seq, entity, row_id, kind, data, timestamp: now_iso8601() };
    // ...
}
```

### 2. Field-level merge on update

When an `update` change lands, the client merges new fields onto the
existing row rather than replacing it wholesale:

```ts
// packages/sync/src/index.ts
case "update":
  const existing = table.get(change.row_id) ?? { id: change.row_id };
  table.set(change.row_id, {
    ...existing,
    ...change.data,
    id: change.row_id,
  });
```

This means **two clients editing disjoint fields of the same row converge
correctly** — Alice setting `Order.notes` and Bob setting `Order.dueDate`
both land. Last-update-wins is per-field, not per-row.

### 3. Last write wins on the same field

When two clients set the *same* field on the same row, the second push to
land on the server wins. There is no per-field timestamp. There is no
vector clock. Whichever push the server processes last is the value
everyone sees.

This is fine for most B2B apps — the natural unit of edit is "one user
clicked Save" and concurrent edits to the same field are rare and
recoverable. It is **not** fine for collaborative text editing, drawing
canvases, or anything where two users routinely touch the same property.

### 4. Tombstones

Deletes record a tombstone with the delete's `seq`. Any subsequent
insert/update for the same `row_id` with `seq < tombstone_seq` is dropped.
This prevents "stale resurrects" — a delayed pre-delete update arriving
after the delete from another client cannot bring the row back.

Optimistic local deletes use `Number.MAX_SAFE_INTEGER` as the tombstone
seq so the client view dominates until the server confirms.

### 5. Optimistic mutations

The React layer's `useMutation` and `db.insert/update/delete` apply
optimistically against the local store, then push to the server. On
failure the optimistic write rolls back. Identifiers for optimistic
inserts are temporary (`_pending_<ts>_<rand>`) and replaced when the
server-issued id arrives.

### 6. Offline write queue

`MutationQueue` persists pending writes through an IndexedDB-backed
adapter (or any user-supplied `MutationQueuePersistence`). Writes made
while offline survive reload and push when connectivity returns.
Idempotent replay is handled by an `op_id` (timestamp + random suffix)
the server tracks per-client.

### 7. Identity-flip resync

When the auth token changes (anonymous → signed in, user A → user B,
sign-out), the server's visible row set changes under the client. The
sync engine resets the local replica + cursor and pulls fresh under the
new identity. Without this, a logged-out user could keep seeing the
previous user's cached rows until the next pull invalidated them.

### 8. Cursor-too-old recovery

If a client's cursor is older than the oldest event the server still
remembers (the change log is bounded), the server returns
`410 RESYNC_REQUIRED`. The client clears its replica and pulls from
seq=0 via the entity-list endpoints (which the server replays as seed
events on start).

## What you do *not* get

- **CRDT semantics.** No Yjs, no Automerge, no per-field HLCs. Two
  clients setting the same field concurrently is a coin flip resolved by
  server arrival order.
- **Multi-device offline merge.** Two devices both offline, both editing
  the same row, can't reconcile without the server. The first to push
  wins; the second's same-field writes are silently overwritten.
- **Causality preservation across writers.** Pylon does not detect that
  Alice's update was made *with knowledge of* Bob's prior update. There
  are no version vectors and no causal-history checks.
- **Operational transforms.** Text fields are bytes — concurrent edits
  to the same string overwrite each other. Don't build Google Docs on
  this.
- **Local-first per Ink & Switch.** The
  [seven principles](https://www.inkandswitch.com/local-first/) — no
  spinners, multi-device, network-optional, longevity, privacy, user
  ownership, no-failure-modes — are not all met. Pylon meets *some* of
  them (offline writes, fast local reads), not all.

## What this is good for

This sync model is well-suited to:

- **B2B SaaS dashboards.** "User opens record, edits, saves." Concurrent
  same-field edits are rare; field-level merge handles the common case.
- **Internal tools.** Same shape — discrete CRUD operations on
  well-defined records.
- **Activity feeds, comments, messages.** Each row is owned by one
  writer; no contention.
- **Real-time presence and notifications.** Pylon's room layer (separate
  from sync) handles ephemeral state; sync handles durable state.
- **Apps that go offline briefly.** A few minutes / hours of disconnect
  followed by reconnect works. Multi-day offline with concurrent edits
  is iffy.

It is **not** suited to:

- **Collaborative text editing.** Use Yjs or Automerge as a separate layer.
- **Whiteboards, drawing tools.** Same — needs a real CRDT.
- **Decentralized / no-central-authority apps.** Pylon requires a server.
- **Long-offline workflows on shared data.** Field-level merge breaks down
  when the offline window is long enough that conflicting same-field
  writes are likely.

## Roadmap

We have three honest options, ordered by effort.

### Option A: Per-field hybrid logical clocks (HLCs)

Replace the implicit "last to land at server" with explicit per-field
HLC timestamps. Each row stores `{ value: ..., hlc: <node_id, ts, counter> }`
per writable field. Merges pick the value with the higher HLC.

**Pros:** No more silent data loss on concurrent same-field writes — the
HLC ordering is deterministic across writers and survives offline merge.
**Cons:** Schema overhead (~20 bytes per field). Requires a one-time
migration. Doesn't help text or list types — still last-write-wins, just
with a defensible tiebreaker.
**Effort:** ~1 week. Server-side: HLC assignment in `change_log.append`.
Client-side: HLC tagging on optimistic writes; HLC-aware merge in
`LocalStore.applyChange`.

This is what we'll ship next if local-first becomes a real customer ask.

### Option B: Yjs adapter as a separate package

Ship `@pylonsync/yjs` that maps a designated `Y.Doc` per entity row onto
the sync engine. Apps opt in per-entity. The Yjs document is the source
of truth for that row's contents; Pylon just transports the binary
update stream.

**Pros:** Real CRDT semantics where you actually need them (text, lists).
The rest of your schema keeps the cheap LWW model.
**Cons:** Adds Yjs as a dependency. Schema migrations get more
interesting. Conflict resolution is now per-field-type, not uniform.
**Effort:** ~2 weeks for a working integration; longer to harden.

Right answer for apps with both "boring CRUD" tables and "rich
collaborative" tables.

### Option C: Full CRDT-native rewrite

Rebuild the sync engine on Automerge or a similar CRDT primitive end to
end. Every row is a CRDT document. Convergence is automatic.

**Pros:** True local-first. Every Ink & Switch principle within reach.
**Cons:** Major architectural change. Performance characteristics flip
(CRDT documents are bigger and slower than JSON). No partial migration
path — the schema layer changes shape. Server's role shrinks
considerably.
**Effort:** Multiple months. Probably better as a separate "Pylon CRDT"
product than a v2 of the existing engine.

## Until then: positioning rules

- Do not say **"local-first"** in marketing or docs. Call it **"optimistic
  + offline-capable sync"** or **"server-authoritative sync with offline
  writes"**.
- Do not say **"no conflicts"**. There are conflicts; the server resolves
  them by arrival order.
- Do not say **"CRDT-backed"** or **"multi-device convergence"**. Both are
  false today.
- Do say **"local-first for the boring 90% of CRUD"** if you need a
  punchy line. It's true and unambiguous.
- Do say **"add Yjs for collaborative text"** when users ask about it —
  honest acknowledgement of the gap is better than over-claiming.
