import {
	entity,
	field,
	query,
	action,
	policy,
	buildManifest,
} from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// Consumer feed schema. The shape:
//
//   Profile  — public-facing card per User. Handle, display name, bio.
//   Post     — one row per post. authorId references the Profile that
//              created it; body holds the text.
//   Like     — pivot row: which Profile liked which Post. Composite
//              uniqueness enforced via the ordering of inserts (one
//              row per (profileId, postId) pair).
//
// Reads are wide-open by design — feeds are public. Writes require
// the caller to own the Profile in question (auth.userId == profileId
// for Profile updates; auth.userId must own a Profile to post).
// ---------------------------------------------------------------------------

const Profile = entity("Profile", {
	// Ownership fields lock with `.readonly()` so HTTP PATCH can't
	// rewrite them. Server-side ctx.db.update inside actions still
	// goes through — readonly is an HTTP-boundary check only.
	userId: field.id("User").readonly(),
	handle: field.string(),
	displayName: field.string(),
	bio: field.string().optional(),
	createdAt: field.datetime().readonly(),
});

const Post = entity("Post", {
	authorId: field.id("Profile").readonly(),
	body: field.string(),
	createdAt: field.datetime().readonly(),
});

const Like = entity("Like", {
	profileId: field.id("Profile").readonly(),
	postId: field.id("Post").readonly(),
	createdAt: field.datetime().readonly(),
});

// ---------------------------------------------------------------------------
// Function declarations
// ---------------------------------------------------------------------------

const myProfile = query("myProfile");
const feed = query("feed");
const profilePosts = query("profilePosts");

const upsertProfile = action("upsertProfile", {
	input: [
		{ name: "handle", type: "string" },
		{ name: "displayName", type: "string" },
		{ name: "bio", type: "string" },
	],
});

const createPost = action("createPost", {
	input: [{ name: "body", type: "string" }],
});

const deletePost = action("deletePost", {
	input: [{ name: "id", type: "id(Post)" }],
});

const toggleLike = action("toggleLike", {
	input: [{ name: "postId", type: "id(Post)" }],
});

// ---------------------------------------------------------------------------
// Policies — public reads, owner-only writes.
// ---------------------------------------------------------------------------

const profilePolicy = policy({
	name: "profile_public",
	entity: "Profile",
	allowRead: "true",
	allowInsert: "auth.userId == data.userId",
	// Update + delete pin `existing.userId` (not the payload) so an
	// attacker can't PATCH `{userId: <them>}` to claim another profile.
	allowUpdate: "auth.userId == existing.userId",
	allowDelete: "auth.userId == existing.userId",
});

const postPolicy = policy({
	name: "post_public",
	entity: "Post",
	allowRead: "true",
	// Insert: caller must own a Profile they're claiming as author.
	allowInsert:
		"exists(Profile where id = data.authorId and userId = auth.userId)",
	// Update + delete pin `existing.authorId` (not the payload) so an
	// attacker can't rewrite the author to grant themselves access.
	allowUpdate:
		"exists(Profile where id = existing.authorId and userId = auth.userId)",
	allowDelete:
		"exists(Profile where id = existing.authorId and userId = auth.userId)",
});

const likePolicy = policy({
	name: "like_public",
	entity: "Like",
	allowRead: "true",
	// Caller must own the Profile doing the liking. Like has no
	// update path (allowUpdate: "false"), so the IDOR-via-PATCH
	// shape doesn't apply — but delete pins `existing.profileId`
	// for consistency with the other policies.
	allowInsert:
		"exists(Profile where id = data.profileId and userId = auth.userId)",
	allowUpdate: "false",
	allowDelete:
		"exists(Profile where id = existing.profileId and userId = auth.userId)",
});

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

const manifest = buildManifest({
	name: "__APP_NAME_SNAKE__",
	version: "0.0.1",
	entities: [Profile, Post, Like],
	queries: [myProfile, feed, profilePosts],
	actions: [
		upsertProfile,
		createPost,
		deletePost,
		toggleLike,
	],
	policies: [profilePolicy, postPolicy, likePolicy],
	routes: [],
});

// Not a debug leftover: the CLI runs `bun run` on this file and parses stdout
// as your manifest (see crates/cli/src/bun.rs). Deleting this line breaks every
// pylon command that reads your schema.
console.log(JSON.stringify(manifest));
