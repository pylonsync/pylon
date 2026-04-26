# Pylon: Sync Semantics

This document describes what Pylon syncs, what converges as a CRDT, and
where the server is still authoritative.

## The honest one-liner

**Pylon is a server-backed realtime sync system with a CRDT row substrate.**
Each CRDT-mode entity row is backed by a Loro document, projected into the
normal SQLite/Postgres row shape for queries and indexes, and broadcast to
clients over the same WebSocket used by live queries.

The important nuance: not every field has the same merge behavior. `richtext`
and fields marked `crdt: "text"` use `LoroText` and merge concurrent text
edits. Most scalar fields use LWW registers inside the row doc because names,
emails, statuses, slugs, and timestamps do not benefit from character-level
merge.

## Two sync layers

### 1. JSON live-query projection

The React `db.useQuery(...)` path consumes ordinary JSON row changes:

1. Client opens a WebSocket to the Pylon server.
2. `useQuery` subscribes to an entity/filter pair.
3. The server runs the query once and sends the current matching rows.
4. Writes append to the change log.
5. Matching subscribers receive row diffs.
6. The client applies diffs to an IndexedDB-backed local replica.

This layer is what powers live dashboards, lists, search results, policies,
pagination, and the normal `db.insert/update/delete` APIs.

JSON mutations are optimistic. The client applies them locally, stores pending
work in the IndexedDB mutation queue, and retries with an `op_id` so server
replays are idempotent. Deletes use tombstones so stale replay cannot
resurrect a row.

### 2. Loro row documents

For CRDT-mode entities, the persisted row is also represented as one Loro doc:

```text
LoroDoc
  map "row"
    fieldName -> LWW scalar, LoroText, or another CRDT container
```

The Rust runtime owns the canonical server copy. On insert/update, Pylon applies
the patch to the Loro doc, projects the doc back to a flat JSON object, stores
that projection in the database, appends a normal change-log event, and sends a
binary CRDT snapshot to subscribed clients.

Clients that need raw CRDT behavior use `@pylonsync/loro`:

```tsx
import { useCollabText } from "@pylonsync/loro";

export function Editor({ noteId }: { noteId: string }) {
  const [body, setBody] = useCollabText("Note", noteId, "body");
  return <textarea value={body} onChange={(e) => setBody(e.target.value)} />;
}
```

`useLoroDoc(entity, id)` subscribes to binary CRDT frames for one row. Local
text edits are encoded as Loro updates and sent to
`POST /api/crdt/<entity>/<row_id>`. The server imports the update, projects the
merged document, and broadcasts the post-merge snapshot back over WebSocket.

## Field merge behavior

Pylon defaults are intentionally mixed:

| Field shape | Default CRDT container | Merge behavior |
|---|---|---|
| `string` | LWW register | Last writer wins |
| `datetime` | LWW string register | Last writer wins |
| `id(Entity)` | LWW string register | Last writer wins |
| `int` / `float` | LWW number register | Last writer wins |
| `bool` | LWW bool register | Last writer wins |
| `richtext` | `LoroText` | Concurrent text edits converge |
| `field.string().crdt("text")` | `LoroText` | Concurrent text edits converge |

Reserved annotations exist for future containers:

| Annotation | Intended container | Status |
|---|---|---|
| `crdt: "counter"` | `LoroCounter` | Reserved, not implemented |
| `crdt: "list"` | `LoroList` | Reserved, not implemented |
| `crdt: "movable-list"` | `LoroMovableList` | Reserved, not implemented |
| `crdt: "tree"` | `LoroTree` | Reserved, not implemented |
| `crdt: "lww"` | LWW register | Implemented |

Setting a reserved annotation today fails rather than silently pretending to
merge.

## Local and offline behavior

Pylon gives clients fast local reads through the IndexedDB mirror and optimistic
JSON writes through the mutation queue.

CRDT text edits are applied to the local Loro doc immediately. The current
`@pylonsync/loro` helper sends the resulting update to the server immediately;
the server broadcast is the durable acknowledgement. If that HTTP push fails,
the helper logs the failure and does not yet persist a CRDT-specific retry
queue. The JSON mutation queue and the CRDT update path are separate.

That means:

- Normal CRUD writes survive reload/offline through the mutation queue.
- CRDT text state converges across connected clients through Loro updates.
- A durable offline queue for unsent CRDT binary updates is still a separate
  hardening item.

## Cursor and resync behavior

The JSON change log uses monotonic server sequence numbers. Clients pull from a
cursor (`since=<last_seq>`) and apply events in order.

If a client's cursor is older than the oldest retained event, the server returns
`410 RESYNC_REQUIRED`. The client clears its local replica and pulls a fresh
seed view. Auth identity changes also trigger a replica reset so one user's
cached rows do not leak into another user's session.

CRDT subscriptions are refcounted by `(entity, row_id)` and resent after WebSocket
reconnect. On subscribe, the server sends the current Loro snapshot for that row.

## What this is good for

- Live SaaS dashboards and internal tools.
- Chat, comments, activity feeds, and presence-backed apps.
- Rich text fields where two clients may edit the same content.
- Apps that want local reads, optimistic CRUD, and CRDT merge where it matters.
- Rows that need search/index/query support while retaining a CRDT source doc.

## What still needs care

- Scalar fields are LWW unless explicitly backed by a richer CRDT container.
- `counter`, `list`, `movable-list`, and `tree` annotations are reserved but not
  implemented yet.
- `useCollabText` currently does whole-text replace operations. It converges via
  Loro, but a lower-level insert/delete API will be better for editors, IME, and
  large documents.
- CRDT updates do not yet have the same durable offline retry queue as JSON
  mutations.
- Pylon still has a canonical server. It is local-first in the product sense of
  local reads and mergeable CRDT state, not a decentralized no-server database.

## Positioning rules

- Do say **"CRDT-backed rows"** or **"Loro-backed row documents"**.
- Do say **"rich text and `crdt(\"text\")` fields merge concurrent edits"**.
- Do say **"server-backed local-first sync"** when explaining the architecture.
- Do not imply every scalar field is conflict-free; most scalar fields are LWW.
- Do not imply CRDT binary updates have durable offline retry until that queue is
  implemented.
