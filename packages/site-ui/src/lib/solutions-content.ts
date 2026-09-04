// Content source of truth for the /solutions/* pages — one use-case vertical
// per entry, keyed by the slug in lib/site-nav.ts. Use-case framing (the
// problem → how Pylon solves it → which primitives do the work), as opposed
// to the feature framing of /product/*. The `primitives` arrays cross-link to
// the relevant /product/<slug> pages.

export interface SolutionCapability {
	title: string;
	body: string;
	/** Product slugs that power this capability. */
	primitives: string[];
}

export interface SolutionContent {
	slug: string;
	navLabel: string;
	category: string;
	title: string;
	tagline: string;
	/** The problem framing, one paragraph. */
	problem: string;
	capabilities: SolutionCapability[];
	/** Primitives the solution leans on, for the "built with" rail. */
	primitives: string[];
	metaTitle: string;
	metaDescription: string;
}

export const SOLUTIONS_CONTENT: Record<string, SolutionContent> = {
	"local-first": {
		slug: "local-first",
		navLabel: "Local-first apps",
		category: "Solution",
		title: "Local-first apps that feel instant.",
		tagline:
			"Reads hit an in-browser store, writes apply optimistically, and everything reconciles with the server in the background. Your app stays responsive offline and syncs when it reconnects, with no sync layer for you to write.",
		problem:
			"Offline-capable apps usually require a local cache, optimistic updates, conflict resolution, and a reconnect protocol. The races between those layers can consume months that should go into the product itself.",
		capabilities: [
			{
				title: "Optimistic writes, reconciled",
				body: "Mutations apply to the local store immediately so the UI never waits on the network. The sync engine tracks each pending op, reconciles against authoritative server state on ack, and rolls back cleanly if a policy rejects it. The race conditions are solved once, in the engine, not re-solved in every component.",
				primitives: ["sync", "functions"],
			},
			{
				title: "Offline queue, automatic catch-up",
				body: "Go offline and writes queue locally; come back and they push in order, then the client pulls anything it missed. A snapshot-plus-delta protocol keeps catch-up cheap even after a long disconnect, and per-op tracking means nothing is double-applied or lost.",
				primitives: ["sync"],
			},
			{
				title: "Multi-tab and multi-device coherent",
				body: "Open the app in three tabs and they share one connection via a leader election over BroadcastChannel, so they stay consistent without three sockets. Across devices, the same typed database is the source of truth, gated by the same row-level policies.",
				primitives: ["sync", "database", "auth"],
			},
		],
		primitives: ["sync", "database", "auth"],
		metaTitle: "Local-first apps on Pylon: instant, offline-ready, auto-synced",
		metaDescription:
			"Build instant, offline-capable apps without writing a sync layer. Optimistic writes, an offline queue, automatic catch-up, and multi-tab coherence ship in the engine.",
	},

	collaboration: {
		slug: "collaboration",
		navLabel: "Realtime collaboration",
		category: "Solution",
		title: "Multiplayer, built in.",
		tagline:
			"Live cursors, presence, and shared state for docs, boards, and dashboards. One server syncs your rows and broadcasts who's online and what they're doing.",
		problem:
			"Adding multiplayer usually means bolting a realtime service onto your database and keeping two systems in sync: one for persistent data, another for ephemeral presence and broadcast. That seam creates stale cursors, ghosted users, and conflicting state.",
		capabilities: [
			{
				title: "Presence and live cursors",
				body: "Join a room, publish a cursor position, selection, or typing indicator, and receive everyone else's presence in real time. Pylon handles join and leave events so users appear and disappear cleanly.",
				primitives: ["realtime"],
			},
			{
				title: "Shared state that persists",
				body: "Ephemeral signals go over broadcast; the document itself is rows in your typed database, synced live to every participant. Because both run on the same server, the persistent state and the presence layer never disagree about who changed what.",
				primitives: ["sync", "realtime"],
			},
			{
				title: "Permissions that hold under concurrency",
				body: "Row-level policies gate every collaborative write in the hot path for each participant. Viewers cannot edit, non-members cannot read, and access changes take effect across connected clients immediately.",
				primitives: ["auth", "functions"],
			},
		],
		primitives: ["realtime", "sync", "auth"],
		metaTitle: "Realtime collaboration on Pylon: presence and multiplayer built in",
		metaDescription:
			"Live cursors, presence, and shared state for collaborative docs and boards. The layer that syncs your data also broadcasts presence, from the same server.",
	},

	"ai-apps": {
		slug: "ai-apps",
		navLabel: "AI apps & agents",
		category: "Solution",
		title: "AI apps with live state.",
		tagline:
			"Call a model from a server function, stream the result into rows your UI is already subscribed to, and run long agent loops as durable workflows that survive restarts. The model, the state, and the realtime layer live in one server.",
		problem:
			"AI products are mostly plumbing: a model call, a place to stream tokens, a way to push partial results to the client, durable state for multi-step agents, and background jobs for the slow parts. Stitched together from a model SDK, a queue, a websocket service, and a database, it's a lot of moving parts to keep coherent.",
		capabilities: [
			{
				title: "Models from a server function",
				body: "ctx.llm calls a model from inside a Pylon function with the same typed db, auth, and schedule context as the rest of your backend. The call sits next to the data it reads and the rows it writes.",
				primitives: ["functions"],
			},
			{
				title: "Stream into live state",
				body: "Write partial results to a row as they arrive and every db.useQuery watching it re-renders token output, status updates, and intermediate steps live. The existing sync engine pushes those changes, so you do not need a separate streaming channel.",
				primitives: ["sync", "functions"],
			},
			{
				title: "Durable agent loops",
				body: "Multi-step agents run as durable workflows that call tools, wait for results, sleep, retry, and continue. Pylon checkpoints every step so a deploy or crash does not lose the run. Background jobs handle slow work after the request returns.",
				primitives: ["workflows", "functions"],
			},
		],
		primitives: ["functions", "sync", "workflows"],
		metaTitle: "AI apps & agents on Pylon: ctx.llm, live state, durable workflows",
		metaDescription:
			"Call models from server functions, stream results into rows your UI already subscribes to, and run durable agent loops as workflows that survive restarts.",
	},

	mobile: {
		slug: "mobile",
		navLabel: "Mobile (Swift)",
		category: "Solution",
		title: "Native iOS and Mac, same backend.",
		tagline:
			"The Swift SDK runs the full Pylon sync engine natively, giving iOS and Mac apps the same live queries, optimistic writes, and offline behavior as the web. Generate a typed Swift client from your schema.",
		problem:
			"Cross-platform usually means the web app and the native app drift: different data layers, different caching, different bugs. A REST SDK on mobile loses the local-first behavior the web client has, and keeping two hand-written sync implementations honest is a losing battle.",
		capabilities: [
			{
				title: "The engine, in Swift",
				body: "packages/swift implements reconciliation, the operation queue, optimistic rollback, and snapshot pagination in Swift. Your iOS and Mac apps get the browser's local-first guarantees through a native sync engine.",
				primitives: ["swift", "sync"],
			},
			{
				title: "Parity, enforced",
				body: "The TypeScript and Swift engines are held at feature parity by policy: every sync fix lands in both. That's what keeps a web app and a native Mac app on one backend from diverging into platform-specific data bugs.",
				primitives: ["swift"],
			},
			{
				title: "Typed from your schema",
				body: "Run pylon codegen client --target swift and get Swift models and a typed client generated from the same schema your backend uses. A Loro CRDT bridge handles rich collaborative state. Your SwiftUI views call the same queries and mutations by name.",
				primitives: ["swift", "database", "realtime"],
			},
		],
		primitives: ["swift", "sync", "realtime"],
		metaTitle: "Mobile on Pylon: the native Swift sync engine for iOS and Mac",
		metaDescription:
			"The Swift SDK runs the full Pylon sync engine with live queries, optimistic writes, and offline support. It stays at parity with TypeScript and generates code from your schema.",
	},
};

export const SOLUTION_SLUGS = Object.keys(SOLUTIONS_CONTENT);

export function getSolution(slug: string): SolutionContent | undefined {
	return SOLUTIONS_CONTENT[slug];
}
