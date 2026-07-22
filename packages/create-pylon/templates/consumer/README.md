# __APP_NAME__

A live social feed on [Pylon](https://pylonsync.com) with a public timeline,
optimistic posts, and likes over one synced backend.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321 and post something. It appears optimistically and
syncs to a second tab along with like counts. Editing a file under `app/`
reloads the page.

## Layout

```
app.ts              data model: Post + Like entities, feed policies, auth
app/page.tsx        "/" — the server-rendered page
app/feed-client.tsx client island: guest session + live feed, posts, likes
app/layout.tsx      root layout wrapping every page
app/globals.css     Tailwind entrypoint (compiled by Pylon)
```

## How it works

No login wall: `app/feed-client.tsx` wraps the feed in `<EnsureGuest>`, which
mints a guest session so every visitor can post + like. The feed is
**public-read** (everyone sees every post and like count — intentional for a
feed), while writes are **owner-only**: `authorId`/`userId: field.owner()`
stamp the session's id server-side, so an optimistic `db.insert` can't forge
authorship. A like is a join row (one per user per post); the count is just how
many `Like` rows point at a post, and `db.useQuery` keeps it live.

## Grow it

- **Profiles:** add a `Profile` entity (displayName/avatar keyed by `userId`)
  to show names instead of `@guest…` handles.
- **Follows:** add a `Follow` join entity (`followerId`/`followedId`) and
  filter the feed to people you follow.
- **Real accounts:** email/password is built in — swap `<EnsureGuest>` for
  `<SignedIn>` / `<SignedOut>` from `@pylonsync/client`.

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
