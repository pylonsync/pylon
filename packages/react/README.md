# @pylonsync/react

React hooks for [Pylon](https://pylonsync.com) — live queries, optimistic mutations, reactive server functions, search, presence, all backed by a local sync replica that re-renders on every change.

Pylon is the open-source application framework from [Stack0](https://stack0.dev).

```sh
bun add @pylonsync/react
# or: npm i @pylonsync/react
```

## Quick start

```tsx
// app entry — once, anywhere before your first hook fires
import { init } from "@pylonsync/react";

init({ baseUrl: "http://localhost:4321" });

// any component
import { db } from "@pylonsync/react";

type Todo = { id: string; title: string; done: boolean };

export function TodoList() {
  const { data: todos, loading } = db.useQuery<Todo>("Todo", {
    where: { done: false },
    orderBy: { createdAt: "desc" },
  });

  const create = db.useMutation<{ title: string }>("createTodo");

  if (loading) return <Spinner />;
  return (
    <>
      <ul>{todos.map((t) => <li key={t.id}>{t.title}</li>)}</ul>
      <button onClick={() => create.mutate({ title: "Buy milk" })}>+ Add</button>
    </>
  );
}
```

`db.useQuery` subscribes to the local sync replica. Inserts (yours or anyone else's), updates, deletes — every component reading this entity re-renders automatically. No queryClient, no manual invalidation, no polling.

## The `db` namespace

`db.*` is the ergonomic API for apps that use a single global sync engine (set up via `init()` once at startup). It owns engine lifecycle, optimistic update bookkeeping, and storage namespacing.

| Method | What it does |
| --- | --- |
| `db.useQuery<T>(entity, opts?)` | Live list of rows |
| `db.useQueryOne<T>(entity, id)` | Live single row |
| `db.useInfiniteQuery<T>(entity, opts)` | Cursor-paginated list with `loadMore()` |
| `db.useReactiveQuery<T>(fnName, args?)` | Convex-style auto-rerunning server query |
| `db.useAggregate<R>(entity, spec)` | Live count / sum / avg / groupBy |
| `db.useSearch<T>(entity, spec)` | Full-text + faceted live search |
| `db.useMutation<I, O>(fnName, opts?)` | Server function call w/ optional optimistic update |
| `db.useEntity(entity)` | Optimistic CRUD bound to one entity |
| `db.insert/update/delete` | Imperative writes (returns id) |
| `db.fn<T>(name, args?)` | Imperative server function call |
| `db.streamFn(name, args?)` | Async iterable of SSE chunks |
| `db.uploadFile(blob, opts?)` | Upload to `/api/files/upload` |
| `db.setPresence(data)` | Publish presence for the current user |
| `db.publishTopic(topic, data)` | Fire-and-forget pubsub |
| `db.sync` | The underlying `SyncEngine` for escape hatches |

## Auth

```tsx
import { useSession } from "@pylonsync/react";

function NavBar() {
  const { auth, signOut } = useSession(db.sync);
  if (!auth) return <a href="/login">Sign in</a>;
  return (
    <>
      <span>{auth.user.displayName}</span>
      <button onClick={signOut}>Sign out</button>
    </>
  );
}
```

`useSession` returns a live `auth` object (re-renders when the user signs in, signs out, switches org, or has their session revoked from another device). It also exposes `selectOrg`, `clearOrg`, and `refresh` for multi-tenant flows.

## Optimistic mutations

```tsx
const send = db.useMutation<
  { channelId: string; body: string },
  { messageId: string }
>("sendMessage", {
  optimistic: (args, ctx) => ({
    entity: "Message",
    data: {
      id: ctx.id,        // ghost id; server's reply merges in-place
      channelId: args.channelId,
      body: args.body,
      authorId: me.id,
      createdAt: ctx.now,
    },
  }),
});

await send.mutate({ channelId, body: "Hi!" });
// Message appears in `db.useQuery("Message", ...)` before the server responds.
```

The framework paints the ghost into the local store, threads `ctx.id` to the server (where your handler should re-use it), and reconciles on server broadcast. See the [optimistic updates concept doc](https://docs.pylonsync.com/concepts/optimistic-updates) for the full pattern.

## Live presence + rooms

```tsx
import { useRoom } from "@pylonsync/react";

function Editor({ documentId }: { documentId: string }) {
  const room = useRoom(db.sync, `doc:${documentId}`);
  return (
    <div>
      {room.peers.map((p) => (
        <Cursor key={p.id} x={p.presence.x} y={p.presence.y} />
      ))}
      <textarea
        onChange={(e) =>
          room.setPresence({ cursor: e.target.selectionStart })
        }
      />
    </div>
  );
}
```

## Configuration

```ts
init({
  baseUrl: "http://localhost:4321",
  // Optional — namespaces localStorage keys so multiple Pylon apps
  // on the same origin don't fight over `pylon:token`.
  appName: "my-app",
});
```

In browser contexts, omitting `baseUrl` falls back to `window.location.origin` — the right answer for Next.js + Vercel deploys that use a `next.config.js` rewrite to proxy `/api/*` to the backend. In SSR it falls back to `http://localhost:4321` (the `pylon dev` default).

## With Next.js

Use [`@pylonsync/next`](https://npmjs.com/package/@pylonsync/next) for App Router server helpers, middleware, and cookie-aware auth. The full deploy story including Vercel env vars and CORS is at [docs.pylonsync.com/operations/vercel](https://docs.pylonsync.com/operations/vercel).

## With React Native

Use [`@pylonsync/react-native`](https://npmjs.com/package/@pylonsync/react-native), which ships the same `db.*` API on top of an AsyncStorage-backed sync replica.

## License

MIT
