import {
	auth,
	buildManifest,
	discoverAppRoutes,
	discoverFunctions,
	entity,
	field,
	policy,
} from "@pylonsync/sdk";

// A photo-sharing backend for the native iOS and Expo clients.
//
//   User    — the account. Email + password sign-in writes it.
//   Profile — the public face of a User: handle, name, bio, avatar.
//   Post    — one photo and a caption.
//   Like, Follow — join rows. A count is how many rows point at a thing.
//   Comment — text on a post.
//
// Everything people post is public to read, so every client can subscribe
// to the whole feed and see new posts, likes, and comments live. Every
// write goes through a function in functions/, which checks the caller's
// session before it writes. Clients cannot write rows directly.

const User = entity(
	"User",
	{
		email: field.string(),
		displayName: field.string().optional(),
		// Written by /api/auth/password/register. Never sent to a client.
		passwordHash: field.string().serverOnly().optional(),
		// The register path stamps these; the entity must declare them.
		avatarColor: field.string().optional(),
		emailVerified: field.datetime().optional(),
		createdAt: field.datetime().defaultNow(),
	},
	{ indexes: [{ name: "by_email", fields: ["email"], unique: true }] },
);

const Profile = entity(
	"Profile",
	{
		userId: field.id("User").readonly(),
		handle: field.string(),
		displayName: field.string(),
		bio: field.string().optional(),
		// `/images/...` for the demo people, `/api/files/<id>` for an upload.
		avatarUrl: field.string().optional(),
		createdAt: field.datetime().readonly(),
	},
	{
		indexes: [
			{ name: "by_user", fields: ["userId"], unique: true },
			{ name: "by_handle", fields: ["handle"], unique: true },
		],
	},
);

const Post = entity(
	"Post",
	{
		authorId: field.id("Profile").readonly(),
		imageUrl: field.string(),
		caption: field.string().optional(),
		createdAt: field.datetime().readonly(),
	},
	{
		indexes: [
			{ name: "by_author", fields: ["authorId"], unique: false },
			{ name: "by_created", fields: ["createdAt"], unique: false },
		],
	},
);

const Like = entity(
	"Like",
	{
		profileId: field.id("Profile").readonly(),
		postId: field.id("Post").readonly(),
		createdAt: field.datetime().readonly(),
	},
	{
		indexes: [
			{ name: "by_post", fields: ["postId"], unique: false },
			// One like per profile per post.
			{ name: "by_profile_post", fields: ["profileId", "postId"], unique: true },
		],
	},
);

const Comment = entity(
	"Comment",
	{
		postId: field.id("Post").readonly(),
		profileId: field.id("Profile").readonly(),
		text: field.string(),
		createdAt: field.datetime().readonly(),
	},
	{ indexes: [{ name: "by_post", fields: ["postId"], unique: false }] },
);

const Follow = entity(
	"Follow",
	{
		followerId: field.id("Profile").readonly(),
		followingId: field.id("Profile").readonly(),
		createdAt: field.datetime().readonly(),
	},
	{
		indexes: [
			{ name: "by_following", fields: ["followingId"], unique: false },
			{ name: "by_pair", fields: ["followerId", "followingId"], unique: true },
		],
	},
);

// A user reads only their own account row. Nothing writes it from a client.
const userPolicy = policy({
	name: "user_self",
	entity: "User",
	allowRead: "auth.userId == data.id",
	allowInsert: "false",
	allowUpdate: "false",
	allowDelete: "false",
});

// Public reads, no client writes. The functions are the write path.
const publicReadOnly = (entityName: string) =>
	policy({
		name: `${entityName.toLowerCase()}_public_read`,
		entity: entityName,
		allowRead: "true",
		allowInsert: "false",
		allowUpdate: "false",
		allowDelete: "false",
	});

// Every non-internal file in functions/ becomes a manifest entry.
const fns = await discoverFunctions();

const manifest = buildManifest({
	name: "__APP_NAME_SNAKE__",
	version: "0.0.1",
	entities: [User, Profile, Post, Like, Comment, Follow],
	queries: fns.queries,
	actions: fns.actions,
	policies: [
		userPolicy,
		publicReadOnly("Profile"),
		publicReadOnly("Post"),
		publicReadOnly("Like"),
		publicReadOnly("Comment"),
		publicReadOnly("Follow"),
	],
	// app/ holds one public web page. Serving it also makes the server serve
	// public/, where the demo photos live.
	routes: await discoverAppRoutes(),
	// Email + password: POST /api/auth/password/register and /login.
	auth: auth(),
});

// Not a debug leftover: the CLI runs `bun run` on this file and parses stdout
// as your manifest (see crates/cli/src/bun.rs). Deleting this line breaks every
// pylon command that reads your schema.
console.log(JSON.stringify(manifest));

export default manifest;
