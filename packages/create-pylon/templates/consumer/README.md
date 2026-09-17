# __APP_NAME__

A photo-sharing app on [Pylon](https://pylonsync.com): a home feed, an
explore grid, post pages with comments, and profiles with followers and
saved posts. Likes, comments, and follows update live in every open tab.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321. The first visit loads twelve demo people with
their posts, likes, comments, and follows, and gives you a guest profile.
Post a photo with Create, like with a double tap, and open a second tab to
watch counts change.

## Layout

```
app.ts                          data model, policies, fonts
app/(app)/layout.tsx            navigation around every page
app/(app)/page.tsx              /            home feed
app/(app)/explore/page.tsx      /explore     every post in a grid
app/(app)/p/[id]/page.tsx       /p/:id       one post and its comments
app/(app)/u/[username]/page.tsx /u/:username a profile
components/social/              the client islands for those pages
lib/social.ts                   usernames, counts, feed rules (pure, tested)
lib/seed.ts                     the demo people and posts
lib/site.ts                     the app's name
functions/                      ensureProfile, updateProfile, createPost,
                                addComment, seedFeed
public/images/                  demo avatars and photos
```

## How it works

- **Sessions.** `components/social/session.tsx` wraps each island in
  `<EnsureGuest>`, so every visitor gets a session without a sign-up form,
  then calls `ensureProfile` to give them a username.
- **Reads are public, writes are yours.** Every entity is readable by
  anyone except `Save`, which only its owner can see. A like, follow, or
  save must carry the caller's own id; the policies in `app.ts` refuse any
  other. Posts, comments, and profile edits go through functions, which
  check image URLs, caption length, and username rules on the server.
- **Photos.** Create uploads the file with
  `uploadFile(file, { visibility: "public" })` from `@pylonsync/react`, so
  every visitor can load it, then calls `createPost` with its URL.
- **Counts.** A like count is how many `Like` rows point at a post. The
  queries are live, so a count changes the moment someone taps.

## Grow it

- **Real accounts:** email and password are built in. Add a sign-in page and
  move a guest's profile to the account on sign-in.
- **Scale:** `components/social/use-social.ts` reads every row, which is fine
  for a demo. For a large network, query per screen with `where` and `limit`.
- **Launch:** delete `functions/seedFeed.ts`, `lib/seed.ts`, and
  `public/images/`, and remove the `seedFeed` call in `session.tsx`.

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
