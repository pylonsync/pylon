import {
	entity,
	field,
	query,
	action,
	policy,
	buildManifest,
} from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// Realtime chat schema. The shape:
//
//   Room    — a chat channel. slug + name.
//   Message — one row per message. roomId + authorId + body.
//
// Public reads inside a room (anyone can browse). Writes require a
// signed-in user. For private rooms in production, add a Membership
// entity and gate Room reads on it (mirrors the b2b pattern).
// ---------------------------------------------------------------------------

const Room = entity("Room", {
	slug: field.string(),
	name: field.string(),
	createdAt: field.datetime(),
});

const Message = entity("Message", {
	// `.readonly()` on identity fields blocks HTTP PATCH from
	// rewriting authorship / message location. Server-side
	// ctx.db.update still goes through inside actions.
	roomId: field.id("Room").readonly(),
	authorId: field.id("User").readonly(),
	authorName: field.string(),
	body: field.string(),
	createdAt: field.datetime().readonly(),
});

// ---------------------------------------------------------------------------
// Function declarations
// ---------------------------------------------------------------------------

const listRooms = query("listRooms");
const roomMessages = query("roomMessages");

const createRoom = action("createRoom", {
	input: [
		{ name: "slug", type: "string" },
		{ name: "name", type: "string" },
	],
});

const sendMessage = action("sendMessage", {
	input: [
		{ name: "roomId", type: "id(Room)" },
		{ name: "body", type: "string" },
		{ name: "authorName", type: "string" },
	],
});

// ---------------------------------------------------------------------------
// Policies — wide-open reads, signed-in writes.
// ---------------------------------------------------------------------------

const roomPolicy = policy({
	name: "room_public",
	entity: "Room",
	allowRead: "true",
	allowInsert: "auth.userId != null",
	allowUpdate: "false",
	allowDelete: "false",
});

const messagePolicy = policy({
	name: "message_public",
	entity: "Message",
	allowRead: "true",
	allowInsert: "auth.userId != null and data.authorId == auth.userId",
	allowUpdate: "false",
	allowDelete: "data.authorId == auth.userId",
});

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

const manifest = buildManifest({
	name: "__APP_NAME_SNAKE__",
	version: "0.0.1",
	entities: [Room, Message],
	queries: [listRooms, roomMessages],
	actions: [createRoom, sendMessage],
	policies: [roomPolicy, messagePolicy],
	routes: [],
});

console.log(JSON.stringify(manifest));
