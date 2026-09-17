import {
  entity,
  field,
  font,
  policy,
  auth,
  buildManifest,
  discoverAppRoutes,
  discoverFunctions,
} from "@pylonsync/sdk";

// A photo-sharing app: profiles, photo posts, likes, comments, follows, and
// saves. Everything people post is public to read. Every write carries the
// writer's id, and the policies below (or the functions, for posts,
// comments, and profiles) check it against the session, so a client cannot
// post, like, or follow as someone else. Guest sessions
// (components/social/session.tsx) let a visitor post without a sign-up form.
//
// The ids are plain fields, not `field.owner()`, because the demo data in
// lib/seed.ts is written for twelve made-up people, and an owner field only
// accepts the caller's own id.
//
//   • Profile — the public face of a user: username, name, bio, avatar.
//   • Post    — one photo and a caption.
//   • Like, Save, Follow — join rows. A count is how many rows point at a thing.
//   • Comment — text on a post.

const Profile = entity(
  "Profile",
  {
    userId: field.string().unique(),
    username: field.string().unique(),
    displayName: field.string(),
    bio: field.string().optional(),
    avatarUrl: field.string().optional(),
    createdAt: field.datetime().defaultNow(),
  },
);

const Post = entity(
  "Post",
  {
    authorId: field.string(),
    imageUrl: field.string(),
    caption: field.string().optional(),
    // "square" (1:1) or "portrait" (4:5): how the feed crops the photo.
    shape: field.string().default("square"),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_created", fields: ["createdAt"], unique: false },
      { name: "by_author", fields: ["authorId"], unique: false },
    ],
  },
);

const Like = entity(
  "Like",
  {
    userId: field.string(),
    postId: field.string(),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_post", fields: ["postId"], unique: false },
      // One like per person per post: a second tap is a no-op at storage.
      { name: "by_user_post", fields: ["userId", "postId"], unique: true },
    ],
  },
);

const Comment = entity(
  "Comment",
  {
    postId: field.string(),
    authorId: field.string(),
    text: field.string(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_post", fields: ["postId"], unique: false }] },
);

const Follow = entity(
  "Follow",
  {
    followerId: field.string(),
    followingId: field.string(),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_following", fields: ["followingId"], unique: false },
      { name: "by_pair", fields: ["followerId", "followingId"], unique: true },
    ],
  },
);

// Saves are private: only the person who saved a post can see that they did.
const Save = entity(
  "Save",
  {
    userId: field.string(),
    postId: field.string(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_user_post", fields: ["userId", "postId"], unique: true }] },
);

// An entity with no policy is denied to clients, so these allow-lists are
// what make the app work. An insert must carry the caller's own id; a delete
// is allowed only on the caller's own row.
//
// Profile, Post, and Comment writes go through functions (ensureProfile,
// updateProfile, createPost, addComment) so usernames, captions, and image
// URLs are checked on the server. Likes, follows, and saves are plain
// inserts and deletes from the client.
const profilePolicy = policy({
  name: "profile_public",
  entity: "Profile",
  allowRead: "true",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const postPolicy = policy({
  name: "post_public",
  entity: "Post",
  allowRead: "true",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "auth.userId == data.authorId",
});

const likePolicy = policy({
  name: "like_public",
  entity: "Like",
  allowRead: "true",
  allowInsert: "auth.userId != null && data.userId == auth.userId",
  allowUpdate: "false",
  allowDelete: "auth.userId == data.userId",
});

const commentPolicy = policy({
  name: "comment_public",
  entity: "Comment",
  allowRead: "true",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "auth.userId == data.authorId",
});

const followPolicy = policy({
  name: "follow_public",
  entity: "Follow",
  allowRead: "true",
  allowInsert: "auth.userId != null && data.followerId == auth.userId && data.followingId != auth.userId",
  allowUpdate: "false",
  allowDelete: "auth.userId == data.followerId",
});

const savePolicy = policy({
  name: "save_private",
  entity: "Save",
  allowRead: "auth.userId == data.userId",
  allowInsert: "auth.userId != null && data.userId == auth.userId",
  allowUpdate: "false",
  allowDelete: "auth.userId == data.userId",
});

// Every non-internal function in functions/ becomes a manifest entry —
// this is what makes them show up in /api/manifest, the OpenAPI spec,
// and `pylon codegen`.
const fns = await discoverFunctions();

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Profile, Post, Like, Comment, Follow, Save],
  queries: fns.queries,
  actions: fns.actions,
  policies: [profilePolicy, postPolicy, likePolicy, commentPolicy, followPolicy, savePolicy],
  auth: auth(),
  // Self-hosted: the build fetches the woff2 files and serves them from this
  // app, so there is no third-party font request.
  fonts: [
    font({
      family: "Inter",
      variable: "--font-sans",
      weights: ["400", "500", "600", "700"],
      subsets: ["latin"],
      display: "swap",
      preload: true,
    }),
    // The wordmark.
    font({
      family: "Instrument Serif",
      variable: "--font-display",
      weights: ["400"],
      styles: ["normal", "italic"],
      subsets: ["latin"],
      display: "swap",
      preload: true,
    }),
  ],
  routes: await discoverAppRoutes(),
});

// Not a debug leftover: the CLI runs `bun run app.ts` and parses stdout as
// your manifest (see crates/cli/src/bun.rs). Deleting this line breaks every
// pylon command that reads your schema.
console.log(JSON.stringify(manifest, null, 2));

export default manifest;
