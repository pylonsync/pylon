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
	userId: field.id("User"),
	handle: field.string(),
	displayName: field.string(),
	bio: field.string().optional(),
	createdAt: field.datetime(),
});

const Post = entity("Post", {
	authorId: field.id("Profile"),
	body: field.string(),
	createdAt: field.datetime(),
});

const Like = entity("Like", {
	profileId: field.id("Profile"),
	postId: field.id("Post"),
	createdAt: field.datetime(),
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
	allowUpdate: "auth.userId == data.userId",
	allowDelete: "auth.userId == data.userId",
});

const postPolicy = policy({
	name: "post_public",
	entity: "Post",
	allowRead: "true",
	// Caller must own a Profile that's being claimed as the author.
	allowInsert:
		"exists(Profile where id = data.authorId and userId = auth.userId)",
	allowUpdate:
		"exists(Profile where id = data.authorId and userId = auth.userId)",
	allowDelete:
		"exists(Profile where id = data.authorId and userId = auth.userId)",
});

const likePolicy = policy({
	name: "like_public",
	entity: "Like",
	allowRead: "true",
	// Caller must own the Profile doing the liking.
	allowInsert:
		"exists(Profile where id = data.profileId and userId = auth.userId)",
	allowUpdate: "false",
	allowDelete:
		"exists(Profile where id = data.profileId and userId = auth.userId)",
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

console.log(JSON.stringify(manifest));
