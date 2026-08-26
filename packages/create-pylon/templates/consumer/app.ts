import {
  entity,
  field,
  policy,
  auth,
  buildManifest,
  discoverAppRoutes,
  discoverFunctions,
} from "@pylonsync/sdk";

// A post in the public feed. `authorId: field.owner()` stamps the signed-in
// (guest) user's id server-side, so an optimistic `db.insert("Post", { text })`
// can't forge authorship. The feed itself is public-read (everyone sees every
// post) — that's intentional for a social feed, NOT the insecure wide-open
// default; writes are still owner-only.
const Post = entity(
  "Post",
  {
    authorId: field.string().owner(),
    text: field.string(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_created", fields: ["createdAt"], unique: false }] },
);

// A like is a join row (one per user per post). To toggle a like the client
// inserts or deletes this row; the like count for a post is just how many
// Like rows point at it. `userId: field.owner()` keeps a like attributable to
// exactly one user.
const Like = entity(
  "Like",
  {
    userId: field.string().owner(),
    postId: field.string(),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_post", fields: ["postId"], unique: false },
      // One like per user per post — the unique index makes a double-like a
      // no-op at the storage layer.
      { name: "by_user_post", fields: ["userId", "postId"], unique: true },
    ],
  },
);

// An entity with no policy is denied to clients by default, so these
// allow-lists are what make the feed work.
// `allowInsert` is `auth.userId != null`, not `== data.authorId`: the owner
// field is stamped by field.owner() *after* the policy check, so it's null at
// insert-time. The stamp still guarantees the new row is owned by the caller,
// and read/update/delete enforce ownership on the persisted row.
const postPolicy = policy({
  name: "post_feed",
  entity: "Post",
  allowRead: "true",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId == data.authorId",
  allowDelete: "auth.userId == data.authorId",
});

const likePolicy = policy({
  name: "like_access",
  entity: "Like",
  allowRead: "true",
  allowInsert: "auth.userId != null",
  allowUpdate: "false",
  allowDelete: "auth.userId == data.userId",
});

// `pylon dev` serves the SSR feed and the API from one port. Guest sessions
// (via `<EnsureGuest>` on the page) let every visitor post + like with no
// login. Natural next steps: a `Profile` entity (displayName/avatar keyed by
// userId) to show names instead of ids, and a `Follow` join entity
// (followerId/followedId) to scope the feed to people you follow.
// Every non-internal function in functions/ becomes a manifest entry —
// this is what makes them show up in /api/manifest, the OpenAPI spec,
// and `pylon codegen`.
const fns = await discoverFunctions();

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Post, Like],
  queries: fns.queries,
  actions: fns.actions,
  policies: [postPolicy, likePolicy],
  auth: auth(),
  routes: await discoverAppRoutes(),
});

// Not a debug leftover: the CLI runs `bun run app.ts` and parses stdout as
// your manifest (see crates/cli/src/bun.rs). Deleting this line breaks every
// pylon command that reads your schema.
console.log(JSON.stringify(manifest, null, 2));

export default manifest;
