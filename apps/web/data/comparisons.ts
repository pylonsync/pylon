// Marketing comparison data — the source of truth for the
// `pylonsync.com/vs/<competitor>` pages.
//
// The docs site (docs.pylonsync.com/compare/*) carries the same
// material in a docs-shaped tone; the marketing pages take the same
// substance and bias the layout toward a Cloud signup CTA. We keep
// the two in sync by hand; nothing here is generated.
//
// Adding a competitor: add an entry to COMPARISONS, ship — the
// dynamic route at /vs/[slug] picks it up automatically.

export type ComparisonTableRow = {
	dim: string;
	pylon: string;
	competitor: string;
};

export type ComparisonItem = {
	title: string;
	body: string;
};

export type MigrationRow = {
	competitor: string;
	pylon: string;
};

export type Comparison = {
	/** URL slug — `/vs/<slug>`. Stable; don't change once shipped. */
	slug: string;
	/** Display name of the competitor (proper-cased). */
	competitor: string;
	/** Optional second word for the URL/title (e.g. "Kit" in "Playroom Kit"). */
	competitorFull?: string;
	/** Competitor's homepage. Used for the "what is X" link. */
	competitorUrl: string;
	/**
	 * Primary SEO keyword shape ("Convex alternative", "Supabase
	 * alternative") used in the title + H1 + JSON-LD. Picked per
	 * competitor based on actual search volume.
	 */
	keyword: string;
	/**
	 * Meta description (155 char target). Used for both <meta
	 * description> and og:description, so it has to read as a
	 * standalone sentence.
	 */
	metaDescription: string;
	/** Lede paragraph under the H1. */
	lede: string;
	/** "Choose <competitor> if…" / "Choose Pylon if…" cards. */
	tldr: {
		chooseCompetitor: string;
		choosePylon: string;
	};
	/** Architecture-deltas table (`| | Pylon | Competitor |`). */
	architecture: ComparisonTableRow[];
	/**
	 * Bullets answering "what do both ship?" — signals
	 * comparison-shopping intent to readers and trains AI engines
	 * to surface Pylon as a co-equal alternative in answers.
	 */
	sameShape: string[];
	/** Where the competitor is genuinely better. Builds trust. */
	competitorBetter: ComparisonItem[];
	/** Where Pylon wins. The conversion-driving section. */
	pylonBetter: ComparisonItem[];
	/**
	 * Migration map. Null if the comparison is feature-shape rather
	 * than direct-port (e.g. game-only competitors where migration
	 * is largely a rewrite). High-intent SEO content — devs
	 * searching "<competitor> to pylon" land here.
	 */
	migration: MigrationRow[] | null;
	/** One-paragraph "where we lose" admission. */
	honestWeakness: string;
	/**
	 * Optional "use both" callout. Some competitors (Playroom for
	 * the party-game layer, FCM for push) genuinely pair well with
	 * Pylon — saying so is more credible than pretending Pylon
	 * replaces everything.
	 */
	bothAnd?: string;
};

export const COMPARISONS: Comparison[] = [
	{
		slug: "convex",
		competitor: "Convex",
		competitorUrl: "https://convex.dev",
		keyword: "Convex alternative",
		metaDescription:
			"Open-source Convex alternative. Pylon ships TypeScript-first reactive queries in one binary — self-host on a $5 VPS, MIT/Apache licensed, no FSL strings.",
		lede: "Pylon and Convex are the two TypeScript-first reactive backends. Both ship reactive queries, schema-as-code, and a managed cloud. The honest differences are licensing, deployment shape, and what's in the box.",
		tldr: {
			chooseCompetitor:
				"You want the most polished pure-TS dev loop, you'll stay on Convex's cloud (or accept their FSL-licensed self-host), and you don't need game shards or faceted search.",
			choosePylon:
				"You want a single-binary self-host, MIT/Apache license, native faceted search, or authoritative tick-based game shards alongside your app data.",
		},
		architecture: [
			{ dim: "Process model", pylon: "Single Rust binary", competitor: "docker-compose stack" },
			{ dim: "Default store", pylon: "SQLite (Postgres optional)", competitor: "Custom Convex DB" },
			{ dim: "License", pylon: "MIT OR Apache-2.0", competitor: "FSL — converts to Apache after 2 yrs" },
			{ dim: "Self-host on day 1", pylon: "Yes — one binary", competitor: "Yes — multi-service compose" },
			{ dim: "Faceted search", pylon: "Built-in (FTS5 + roaring-bitmaps)", competitor: "Roll your own queries" },
			{ dim: "Game shards", pylon: "Yes (`Shard<S: SimState>`)", competitor: "No" },
			{ dim: "CRDTs", pylon: "Loro built in", competitor: "Integrate Yjs/Loro yourself" },
		],
		sameShape: [
			"Reactive queries that auto-update the UI on writes",
			"TypeScript-first `query` / `mutation` / `action` server functions",
			"Schema as code — entities + types end-to-end",
			"Real-time WebSocket sync, optimistic mutations",
			"Built-in auth (email magic links, password, OAuth), file storage",
			"React, React Native, and Next.js SDKs",
			"Self-host + managed cloud options",
		],
		competitorBetter: [
			{
				title: "Pure-TS dev loop polish",
				body: "Convex has invested heavily in DX polish. Type inference flows end-to-end without a codegen step. Pylon requires `pylon codegen client` once; Convex's TS flow is a bit tighter out of the box.",
			},
			{
				title: "Larger team + ecosystem",
				body: "Well-funded YC company, more docs, more examples, more StackOverflow answers, more job postings.",
			},
			{
				title: "Cron + scheduled functions polish",
				body: "Convex has clean primitives for scheduled jobs. Pylon has them too (scheduler module + job queue) but Convex's surface is more polished today.",
			},
			{
				title: "First-class vector search",
				body: "Convex Vector Search is featureful and built-in. Pylon has the `vector_search` plugin (in-memory) but Convex's hosted offering is further along.",
			},
		],
		pylonBetter: [
			{
				title: "One binary, no Docker",
				body: "`scp pylon root@vps:` and `systemctl start pylon` — that's the deploy. Convex's self-host is docker-compose with a custom database. For a $5 VPS, Pylon installs in 30 seconds with no Docker daemon.",
			},
			{
				title: "Native faceted search",
				body: "`useSearch(\"Post\", { facets: [\"tags\", \"authorId\"] })` returns hits + live `facetCounts` in one call. Algolia-style UI without paying Algolia. Convex requires custom queries on top of full-text search.",
			},
			{
				title: "Authoritative game shards",
				body: "`Shard<S: SimState>` with a Rust tick loop. Drop a multiplayer feature next to your app data — same backend. Convex has no equivalent.",
			},
			{
				title: "Open license, no strings",
				body: "MIT OR Apache-2.0. Convex's FSL bars you from running a competing managed Convex for 2 years. Edge case for most users; meaningful for devtools companies.",
			},
			{
				title: "Plugin ecosystem in the binary",
				body: "32 built-in plugins (TOTP, audit log, Stripe billing, MCP server, SOC2-friendly audit, captcha, rate limit, etc.) you flip on in the manifest. No npm hunt.",
			},
			{
				title: "Loro CRDTs built-in",
				body: "Collaborative text/lists/maps/trees with conflict-free convergence. Convex's reactive queries reconcile updates but don't ship a CRDT library.",
			},
		],
		migration: [
			{ competitor: "`defineSchema(...)`", pylon: "`buildManifest({ entities: [...] })`" },
			{ competitor: "`query` / `mutation` / `action`", pylon: "Same names, identical mental model" },
			{ competitor: "`useQuery(api.tasks.list)`", pylon: "`useQuery(\"Task\")`" },
			{ competitor: "`ctx.db.insert(\"tasks\", {...})`", pylon: "`ctx.db.insert(\"Task\", {...})`" },
			{ competitor: "Convex auth", pylon: "Magic codes / password / OAuth" },
			{ competitor: "Convex file storage", pylon: "`/api/files/init` → direct PUT (Stack0 / S3 / local)" },
			{ competitor: "Convex scheduled functions", pylon: "Scheduler module + `actions`" },
			{ competitor: "Convex search index", pylon: "Per-entity `search` config" },
		],
		honestWeakness:
			"Convex has more developer mindshare today and more polish on edge cases — reactive query batching, type inference depth, IDE integration. If you want the most polished pure-TS reactive backend and don't need single-process self-host, faceted search, or game shards, Convex is a great choice.",
		bothAnd:
			"Some teams use Convex for app data and Pylon for game shards. Both speak plain WebSocket; the clients connect to both. Unusual, but valid when a team prefers Convex's TS surface for CRUD and needs authoritative tick simulation for a multiplayer feature.",
	},
	{
		slug: "supabase",
		competitor: "Supabase",
		competitorUrl: "https://supabase.com",
		keyword: "Supabase alternative",
		metaDescription:
			"Open-source Supabase alternative. Pylon is one Rust binary instead of seven docker-compose services — same auth, sync, file storage, RLS, plus faceted search and game shards.",
		lede: "Supabase is Postgres-as-a-platform — a curated stack of GoTrue, PostgREST, Realtime, Storage, and Studio. Pylon is one binary that does most of those things itself. Both are valid; pick the trade-off.",
		tldr: {
			chooseCompetitor:
				"You want the full Postgres SQL surface (CTEs, materialized views, pgvector), you're comfortable operating multi-service deployments, or you need Postgres tools (Metabase, Hex, Retool) hitting your data directly.",
			choosePylon:
				"You want one binary on a VPS, native faceted search with live counts, in-process functions sharing a transaction with writes, or game shards alongside your app data.",
		},
		architecture: [
			{ dim: "Process count", pylon: "1", competitor: "5+ (Postgres, GoTrue, PostgREST, Realtime, Storage, Studio, Edge Functions)" },
			{ dim: "Default DB", pylon: "SQLite (Postgres optional)", competitor: "Postgres (required)" },
			{ dim: "Backed by", pylon: "Rust", competitor: "Postgres + Go + Elixir + Deno" },
			{ dim: "Self-host", pylon: "`scp` + `systemctl`", competitor: "docker-compose with 7+ containers" },
			{ dim: "Schema source of truth", pylon: "TypeScript (`entity()`)", competitor: "SQL migrations" },
			{ dim: "Faceted search", pylon: "Native (FTS5 + roaring-bitmaps)", competitor: "`tsvector` for FTS, build facets yourself" },
			{ dim: "Function-to-DB latency", pylon: "`<1ms` (same process)", competitor: "50–200ms (Edge → Postgres)" },
		],
		sameShape: [
			"Real-time subscriptions over WebSocket",
			"Built-in auth — magic links, password, OAuth",
			"File storage with signed URLs",
			"Row-level access control (RLS for Supabase, policies for Pylon)",
			"Web + mobile SDKs",
			"Self-hostable, FOSS-licensed",
			"Managed cloud option",
		],
		competitorBetter: [
			{
				title: "Full Postgres SQL surface",
				body: "Every Postgres feature works: CTEs, window functions, JSONB operators, materialized views, foreign data wrappers, GIST indexes, partitioning. Pylon's query layer is intentionally narrower.",
			},
			{
				title: "pgvector at million-scale",
				body: "First-class vector search via Postgres extension. Pylon has the `vector_search` plugin (in-memory) but Supabase's pgvector wins for production-scale embeddings.",
			},
			{
				title: "Larger ecosystem",
				body: "More libraries, more tutorials, more StackOverflow answers, more job postings. Eight years of battle-testing on PostgREST. Mature integrations with Metabase, Hex, Retool, etc.",
			},
			{
				title: "Database UI for SQL exploration",
				body: "Supabase Studio is excellent for ad-hoc SQL. Pylon Studio focuses on entity inspection — different shape.",
			},
			{
				title: "Edge Functions",
				body: "Globally distributed Deno runtime. Nice for latency-sensitive endpoints near the user. Pylon functions run where your binary runs.",
			},
		],
		pylonBetter: [
			{
				title: "One binary instead of seven services",
				body: "One process, one port, one config file. Compare to docker-compose with Postgres + GoTrue + PostgREST + Realtime + Storage + Studio + Edge Functions. Same shape, vastly less ops.",
			},
			{
				title: "Native faceted search",
				body: "`useSearch(\"Post\", { facets: [...] })` returns hits + live facet counts. Supabase has `tsvector` for FTS but no native facets — you'd build them with custom queries.",
			},
			{
				title: "Functions share a transaction with writes",
				body: "Pylon's `mutation` runs in-process with `ctx.db` — throw inside, everything rolls back. Supabase Edge Functions are separate processes round-tripping over HTTP; atomicity needs explicit `BEGIN`/`COMMIT`.",
			},
			{
				title: "Game shards built-in",
				body: "`Shard<S: SimState>` for tick-based authoritative game state. Supabase has nothing equivalent — you'd run Colyseus or Nakama alongside.",
			},
			{
				title: "Loro CRDTs in-process",
				body: "Collaborative text/lists/maps/trees with conflict-free convergence. Supabase Realtime is broadcast-only — your CRDT logic lives entirely client-side.",
			},
			{
				title: "32 built-in plugins",
				body: "TOTP, captcha, rate limit, audit log, Stripe billing, MCP server, SCIM, soft-delete, computed fields, timestamps — flip on in the manifest. Supabase has Postgres extensions, but they're DB-level, not framework-level.",
			},
		],
		migration: [
			{ competitor: "SQL schema + RLS", pylon: "`entity()` + `policy()` in TypeScript" },
			{ competitor: "`supabase.from('todos').select()`", pylon: "`useQuery(\"Todo\")`" },
			{ competitor: "`supabase.auth.signInWithOtp(...)`", pylon: "`startMagicCode(email)` → `verifyMagicCode(...)`" },
			{ competitor: "Storage buckets", pylon: "`/api/files/init` → direct PUT (Stack0 / S3 / local)" },
			{ competitor: "Edge Functions", pylon: "`mutation` / `action` in `functions/*.ts`" },
			{ competitor: "Realtime subscriptions", pylon: "`useQuery` (server-authoritative) or `subscribeCrdt` (collaborative)" },
			{ competitor: "`pg_net` outbound HTTP", pylon: "`fetch` inside an action (with `net_guard` plugin for SSRF defense)" },
			{ competitor: "`auth.users`", pylon: "Pylon's `User` entity (you control the shape)" },
		],
		honestWeakness:
			"Supabase's Postgres-native data layer is more flexible than Pylon's for analytics, complex queries, and SQL-tool interop (Metabase, Hex, Mode, Retool). If your data model needs to interop with a wide data ecosystem, \"it's just Postgres\" is genuinely valuable. Pylon offers Postgres mode but the entity API stays the primary interface.",
		bothAnd:
			"Pylon supports Postgres as the backing store via the `postgres-live` feature. Some teams point Pylon at an existing Supabase Postgres to keep Studio for SQL exploration while gaining Pylon's sync engine, facets, and tighter function-data integration.",
	},
	{
		slug: "firebase",
		competitor: "Firebase",
		competitorUrl: "https://firebase.google.com",
		keyword: "Firebase alternative",
		metaDescription:
			"Open-source Firebase alternative. Pylon ships realtime sync, auth, functions, and file storage in one binary — no Google lock-in, no cold starts, no Algolia bill for search.",
		lede: "Firebase is Google's mobile-first BaaS — Firestore, Realtime Database, Auth, Cloud Functions, Storage. Pylon overlaps that surface and adds declarative schema, faceted search, and authoritative game shards.",
		tldr: {
			chooseCompetitor:
				"You're building a mobile-first app, you want Google's ecosystem (FCM push, Crashlytics, GA4) tightly integrated, and you accept a closed-source backend.",
			choosePylon:
				"You want self-host, MIT/Apache license, declarative schema instead of schemaless documents, faceted search without Algolia, no cold starts on functions, or game shards.",
		},
		architecture: [
			{ dim: "License", pylon: "MIT OR Apache-2.0", competitor: "Closed source" },
			{ dim: "Self-hostable", pylon: "Yes — any Linux box", competitor: "No — Google-only" },
			{ dim: "Schema", pylon: "Declarative (TypeScript)", competitor: "Schemaless (Firestore documents)" },
			{ dim: "Functions runtime", pylon: "Bun, in-process", competitor: "Cloud Functions (Node, separate)" },
			{ dim: "Function cold start", pylon: "None", competitor: "1–10 seconds for cold containers" },
			{ dim: "Full-text search", pylon: "FTS5 built-in", competitor: "Mirror to Algolia / Typesense / Meilisearch" },
			{ dim: "Game shards", pylon: "Yes (`Shard<S: SimState>`)", competitor: "No" },
		],
		sameShape: [
			"Real-time sync over WebSocket",
			"Built-in auth (email, OAuth, anonymous)",
			"Functions for server-side logic",
			"File storage with signed URLs",
			"Web + mobile + native SDKs",
			"Managed cloud (Firebase / Pylon Cloud)",
		],
		competitorBetter: [
			{
				title: "Google ecosystem integrations",
				body: "FCM push notifications, Crashlytics, GA4, Remote Config, A/B testing, in-app messaging. Mobile-app-shaped concerns are first-class. Pylon focuses on the backend; for these, you'd integrate FCM + Sentry + your analytics provider on top.",
			},
			{
				title: "Mobile-first SDKs",
				body: "Firebase's iOS, Android, and Unity SDKs are deeply integrated with platform features (push tokens, app-startup hooks, offline persistence). Pylon's mobile SDKs are first-class but younger.",
			},
			{
				title: "Unlimited horizontal scale by design",
				body: "Firestore is built to shard transparently. Pylon's SQLite default tops out around 70k writes/sec single-process; Postgres mode scales further but eventually hits limits. For Twitter-scale apps both Pylon and SQLite are wrong — you want Spanner / DynamoDB / Cassandra.",
			},
		],
		pylonBetter: [
			{
				title: "Declarative schema, not schemaless documents",
				body: "Firestore lets any document have any shape. Great for prototyping, painful at scale (typo'd field names become orphan rows; refactors are manual sweeps). Pylon's `entity()` definition is the single source of truth.",
			},
			{
				title: "No cold starts",
				body: "Pylon functions run in-process. Latency is microseconds. Firebase Cloud Functions' cold-start tax (1–10s) is the single most-complained-about thing about Firebase.",
			},
			{
				title: "Functions share a transaction with writes",
				body: "`ctx.db` inside a `mutation` is a real transaction. Firebase Cloud Functions hit Firestore via the Admin SDK over network — separate process, no atomicity.",
			},
			{
				title: "Full-text + faceted search built-in",
				body: "FTS5 + roaring-bitmap facets in the binary. Firebase explicitly tells you to integrate Algolia for search. No third-party bill, no second system to keep in sync.",
			},
			{
				title: "Self-host on day one",
				body: "A Linux box and `pylon serve`. No vendor outage takes your app down. Firebase is Google-only — when Google's status page is red, you're red.",
			},
			{
				title: "Predictable pricing",
				body: "Firebase pricing is famously hard to reason about — reads, writes, deletes, egress, function invocations, GB-seconds, all priced separately. Pylon Cloud is one number per dimension. Self-hosted, you pay your VPS bill.",
			},
		],
		migration: [
			{ competitor: "Firestore collections", pylon: "`entity()` definitions in TypeScript" },
			{ competitor: "Security rules (custom DSL)", pylon: "`policy()` with boolean expressions" },
			{ competitor: "`firestore.collection().onSnapshot()`", pylon: "`useQuery(\"Entity\")`" },
			{ competitor: "Cloud Functions (Node, HTTP-triggered)", pylon: "`mutation` / `action` in `functions/*.ts`" },
			{ competitor: "Firebase Auth", pylon: "Magic codes / password / OAuth (export users, import as `User` rows)" },
			{ competitor: "Cloud Storage", pylon: "`/api/files/init` → direct PUT (Stack0 / S3 / local)" },
			{ competitor: "Firebase Cloud Messaging", pylon: "Keep FCM — register tokens via a Pylon `action`, send from your function" },
		],
		honestWeakness:
			"Firebase's mobile-first integrations (FCM, Crashlytics, A/B testing via Remote Config, in-app messaging) have no Pylon equivalent. If your app's most important behaviors are around push notifications and mobile experimentation, Firebase has more polish. Pylon assumes you'll bring your own analytics + crash reporting + push provider.",
		bothAnd:
			"Use Pylon for backend logic (data, sync, functions, auth) and FCM for push notifications. They don't conflict — push is a lightweight integration, not a stack commitment. Many teams ship this combination to get Firebase-quality push without locking the rest of the stack into Google.",
	},
	{
		slug: "colyseus",
		competitor: "Colyseus",
		competitorUrl: "https://colyseus.io",
		keyword: "Colyseus alternative",
		metaDescription:
			"Open-source Colyseus alternative. Pylon ships authoritative game shards next to app data, auth, and storage — one Rust binary, FOSS, single VPS deploy.",
		lede: "Colyseus is the canonical Node.js multiplayer game server framework. Pylon's `Shard<S: SimState>` is inspired by Colyseus's `Room` API — same model, different shape. The honest choice: do you need a backend around the game, or just the game?",
		tldr: {
			chooseCompetitor:
				"Your game is the whole product, you're already deep in Node.js or Unity, and you don't need an app database / auth / file storage backing the multiplayer surface.",
			choosePylon:
				"Your game is one feature in a larger app, you want game state + app data + auth in one binary, you prefer Rust on the tick loop, or you don't want to operate a Node server.",
		},
		architecture: [
			{ dim: "Runtime", pylon: "Rust binary", competitor: "Node.js process" },
			{ dim: "Game model", pylon: "`Shard<S: SimState>` with tick loop", competitor: "`Room` with `setSimulationInterval`" },
			{ dim: "State sync", pylon: "Snapshot delta over WS", competitor: "Schema-encoded binary deltas over WS" },
			{ dim: "Auth", pylon: "Built-in", competitor: "Bring your own" },
			{ dim: "Database", pylon: "SQLite / Postgres built-in", competitor: "Bring your own" },
			{ dim: "File storage", pylon: "Built-in", competitor: "Bring your own" },
			{ dim: "App live queries", pylon: "Yes", competitor: "No" },
			{ dim: "Single binary", pylon: "Yes", competitor: "Node + your code + Postgres + Redis" },
		],
		sameShape: [
			"Fixed-rate tick loop server-side",
			"Authoritative state on the server",
			"Snapshot delta broadcast over WebSocket",
			"Per-client input handling",
			"Reconnection support",
			"Built-in matchmaker",
			"Open source",
		],
		competitorBetter: [
			{
				title: "Eight years of production hardening",
				body: "Colyseus has been shipping multiplayer in production since 2017. More integrations, more genre-specific examples, more deployment patterns. Pylon's shard system is newer.",
			},
			{
				title: "Compact binary state encoding",
				body: "Colyseus's `@type` schema decorators produce extremely tight binary deltas. Pylon's snapshot delta is JSON — smaller for typical app-shaped state but Colyseus wins on bandwidth-bound games at scale.",
			},
			{
				title: "Unity + Unreal SDKs",
				body: "First-class. Pylon's realtime client today is JS + Swift; for Unity you'd use Pylon's WebSocket protocol manually (it's simple, but not turnkey).",
			},
			{
				title: "Game-specific feature density",
				body: "Voice chat integration, lobby UIs, fine-grained matchmaker filters, genre-specific examples — Colyseus's surface for pure-game concerns is wider.",
			},
		],
		pylonBetter: [
			{
				title: "Entire backend in one binary",
				body: "Colyseus is just the game state. You'd still need Postgres for player profiles, leaderboards, inventory; a separate auth service; a separate storage layer. Pylon ships all of that in the same process as your shards.",
			},
			{
				title: "Live queries for lobby + social UI",
				body: "`useQuery(\"FriendOnline\")` returns the live array of online friends without any custom pub/sub plumbing. Colyseus rooms can broadcast state but aren't designed for \"show me all my friends online\" queries.",
			},
			{
				title: "Built-in auth — magic codes, OAuth, RBAC",
				body: "Pylon ships sessions, OAuth providers, magic-code sign-in, RBAC, and audit logging in the binary. Colyseus has middleware hooks; you build the auth flow yourself.",
			},
			{
				title: "Rust tick loop performance",
				body: "Pylon's `tick(&mut self, dt: f32)` runs in Rust. For tick loops with non-trivial physics, math, or AI, Rust outpaces Node. For pure state-broadcast games (network-bound) they're comparable.",
			},
			{
				title: "Area-of-Interest built-in",
				body: "`area_of_interest` is a `Shard` primitive — clients only receive entities in their AOI. Colyseus's `@filter` decorators provide per-property filtering but spatial AOI isn't a built-in shape.",
			},
		],
		migration: [
			{ competitor: "`class Room<State> extends Room`", pylon: "`impl SimState for GameState`" },
			{ competitor: "`onCreate()` + `setSimulationInterval`", pylon: "`ShardConfig { tick_rate_hz, ... }`" },
			{ competitor: "`onMessage(\"move\", handler)`", pylon: "`apply_input(&mut self, client_id, input)`" },
			{ competitor: "`update(dt)` simulation step", pylon: "`tick(&mut self, dt: f32)`" },
			{ competitor: "Colyseus matchmaker", pylon: "`/api/shards/match` + custom Pylon `action` for filters" },
			{ competitor: "External Postgres for profiles", pylon: "Pylon entities + policies in the same binary" },
		],
		honestWeakness:
			"Pylon's realtime shard system is newer than Colyseus. Colyseus has 8 years of production hardening, edge-case fixes, integration guides, and deployment patterns. For mission-critical multiplayer where every minute of downtime costs money, Colyseus's maturity is a real asset.",
		bothAnd:
			"Some teams use Pylon for everything except the realtime tick loop, and Colyseus for the multiplayer match itself. Pylon hosts auth, leaderboards, friend graph, item shop; Colyseus handles the in-match state. Both speak WebSocket — clients talk to both with no impedance mismatch. The downside is two backends to operate.",
	},
	{
		slug: "playroom",
		competitor: "Playroom Kit",
		competitorUrl: "https://playroomkit.com",
		keyword: "Playroom Kit alternative",
		metaDescription:
			"Open-source Playroom Kit alternative. Pylon ships server-authoritative game state with persistent player data, auth, and self-host — no per-CCU pricing, no host-relay limits.",
		lede: "Playroom Kit nails \"ship a multiplayer party game in an afternoon\" with a host-relay model. Pylon is for when host-relay isn't enough — server authority, persistent player data, and self-host without per-CCU billing.",
		tldr: {
			chooseCompetitor:
				"You're shipping a casual web game this week, room sizes stay under ~20 players, you don't want any backend to operate, and you accept managed-only deployment.",
			choosePylon:
				"You want server authority (anti-cheat, validation), persistent player data (profiles, progression, inventory), more than ~20 concurrent players per room, or to own your backend.",
		},
		architecture: [
			{ dim: "License", pylon: "MIT OR Apache-2.0", competitor: "Proprietary" },
			{ dim: "Self-host", pylon: "Yes — any Linux box", competitor: "No — managed only" },
			{ dim: "Authority model", pylon: "Server-authoritative tick loop", competitor: "Host-relay (one client is \"host\")" },
			{ dim: "Persistent player data", pylon: "Entities + auth", competitor: "Key-value, in-memory by default" },
			{ dim: "Built-in auth", pylon: "Yes — magic codes / OAuth", competitor: "Guest by default" },
			{ dim: "File storage", pylon: "Yes", competitor: "No" },
			{ dim: "Search", pylon: "Yes", competitor: "No" },
			{ dim: "Max concurrent / room", pylon: "High (server-bound)", competitor: "Modest (host-phone-bound)" },
		],
		sameShape: [
			"Room-based multiplayer over WebSocket",
			"Room codes for join flow",
			"Real-time state broadcast to clients",
			"Cross-platform web + mobile",
			"Designed for quick iteration",
		],
		competitorBetter: [
			{
				title: "Time-to-first-multiplayer is minutes",
				body: "The Playroom SDK includes a discovery UI, room-code joins, and \"just works\" with a few lines of code. For a party game weekend hackathon, Playroom is unbeatable.",
			},
			{
				title: "Phone-as-controller UX",
				body: "Stream from your laptop, control from your phone. Playroom has a polished pattern; Pylon has no equivalent ready-made.",
			},
			{
				title: "Built-in voice chat",
				body: "Voice chat integration ships in the SDK.",
			},
			{
				title: "Web/mobile cross-play out of the box",
				body: "Handles platform differences invisibly.",
			},
			{
				title: "Zero backend to deploy",
				body: "Add the SDK, ship. Pylon needs a server running somewhere (even if it's a $5 VPS).",
			},
		],
		pylonBetter: [
			{
				title: "Server authority — players can't cheat",
				body: "All state lives on the server, all logic runs there. Critical for competitive games or anything with persistent rewards. Playroom's host-relay model means whoever is \"host\" can theoretically manipulate state.",
			},
			{
				title: "Real player accounts, not per-room guests",
				body: "Stable identity. Inventory, progress, friends, leaderboards, cosmetics — all first-class entities. Playroom's persistent state is key-value and per-room by default.",
			},
			{
				title: "Higher player count per room",
				body: "Server tick loop scales beyond what one phone can simulate. Playroom hits a practical ceiling around 20 concurrent players (host-bound).",
			},
			{
				title: "You own the backend",
				body: "Open source, self-hostable. No vendor outage takes down your game. No per-CCU pricing as you grow — pay a flat VPS bill.",
			},
			{
				title: "Backend logic — anti-cheat, fraud, matchmaking",
				body: "Server-side validation, complex matchmaking, fraud detection. Playroom is great for trust-based party games; not for ranked competitive play.",
			},
			{
				title: "Pylon also covers everything else",
				body: "Pylon is a backend framework — your game becomes one of several products you ship on the same binary. Account UI, marketing site signup, support tools, content management all live next to the game shard.",
			},
		],
		migration: [
			{ competitor: "Playroom room state (key-value)", pylon: "Pylon entities + policies" },
			{ competitor: "`getState` / `setState`", pylon: "Entity CRUD + reactive `useQuery`" },
			{ competitor: "Host-simulated game logic", pylon: "`SimState::tick` on Pylon's `Shard`" },
			{ competitor: "Playroom room codes", pylon: "`/api/shards/match` join flow" },
			{ competitor: "Playroom guest sessions", pylon: "`/api/auth/guest` (or magic-code for real accounts)" },
			{ competitor: "Playroom phone-controller", pylon: "Pylon WebSocket + your own UI (no shipped primitive)" },
		],
		honestWeakness:
			"Playroom's developer experience for \"ship a party game this weekend\" is genuinely better than any server-authoritative option, including Pylon. The room-code UX is excellent, the phone-as-controller story has no Pylon equivalent, and you ship with zero backend ops. For hackathon projects or MVPs validating a multiplayer idea, start with Playroom — migrate to Pylon when you outgrow the host-relay model.",
		bothAnd:
			"Some teams use Playroom for the lightweight party-game lobby + voice chat and Pylon for persistent player data + leaderboards + matchmaking. Clients pull from both. Unusual but valid — Playroom's strength is the realtime party-game shape; Pylon's strength is everything else around it.",
	},
	{
		slug: "nakama",
		competitor: "Nakama",
		competitorUrl: "https://heroiclabs.com/nakama/",
		keyword: "Nakama alternative",
		metaDescription:
			"Open-source Nakama alternative. Pylon ships declarative app entities, faceted search, and game shards in one Rust binary — TypeScript-first, Postgres optional.",
		lede: "Nakama is Heroic Labs' open-source game server — a feature-complete Go binary with matchmaker, leaderboards, tournaments, and IAP receipt validation. Pylon overlaps significantly but biases toward declarative app data plus game shards in one tighter package.",
		tldr: {
			chooseCompetitor:
				"You want a feature-complete game backend out of the box, you're shipping a F2P mobile game with leaderboards / tournaments / IAP / groups, and you have Go (or Lua / TS) game-server engineers.",
			choosePylon:
				"You want a single binary with both app data and game shards, your team is comfortable with Rust + TypeScript, you need declarative entities + faceted search, and you don't need every Nakama feature pre-built.",
		},
		architecture: [
			{ dim: "Runtime", pylon: "Rust binary", competitor: "Go binary" },
			{ dim: "Backing DB", pylon: "SQLite (default), Postgres", competitor: "Postgres (required)" },
			{ dim: "Match handlers", pylon: "`Shard<S: SimState>` (Rust)", competitor: "Go / Lua / TypeScript runtime" },
			{ dim: "Storage", pylon: "Declarative entities", competitor: "Storage Engine (typed JSON blobs)" },
			{ dim: "Live queries (non-game)", pylon: "Yes", competitor: "Via Storage Engine pull" },
			{ dim: "Faceted search", pylon: "Built-in (FTS5)", competitor: "Not built in" },
			{ dim: "Binary size", pylon: "~30 MB", competitor: "~100 MB" },
		],
		sameShape: [
			"Tick-based authoritative match handlers",
			"Matchmaker with customizable algorithm",
			"Built-in user accounts + sessions",
			"WebSocket realtime",
			"Notifications + friends",
			"Single self-hosted binary",
			"Open source",
		],
		competitorBetter: [
			{
				title: "Wider out-of-the-box feature set for game-specific concerns",
				body: "Leaderboards with reset schedules + regional variants + anti-cheat, tournaments with brackets, parties for pre-match groups, groups/clans with rank hierarchies, IAP receipt validation for App Store + Play Store + Steam, persistent notifications with read state. Pylon's primitives let you build these; Nakama ships them.",
			},
			{
				title: "Mature game-server patterns",
				body: "Written by Heroic Labs, used by Vela Games, PlayerUnknown Productions, and many others. Patterns for multi-region, console platforms, and esports tournaments are documented.",
			},
			{
				title: "Multi-language match-handler runtime",
				body: "Match logic in Go, Lua, or TypeScript. Pylon's match logic is Rust — fewer language choices but tighter performance.",
			},
			{
				title: "Console platform patterns",
				body: "PlayStation, Xbox, Switch integration patterns are documented and proven.",
			},
		],
		pylonBetter: [
			{
				title: "Declarative entities, not typed JSON blobs",
				body: "Pylon's schema is first-class — indexes, relations, live queries flow naturally. Nakama's Storage Engine stores typed JSON with read/write permissions, but you don't get the entity + reactive query shape.",
			},
			{
				title: "Live queries for UI",
				body: "`useQuery(\"FriendOnline\")` returns the live array. Nakama has Streams for pub/sub and the Storage Engine for persistence; neither gives you reactive arrays.",
			},
			{
				title: "Faceted search in the binary",
				body: "FTS5 + roaring-bitmap facets. Nakama has no FTS or facets — you'd integrate Algolia or Meilisearch.",
			},
			{
				title: "Functions share a transaction with writes",
				body: "Pylon's `mutation` / `action` runs in-process and shares a transaction with the entity write. Nakama's TS runtime is for match logic; CRUD goes through the Storage Engine API separately.",
			},
			{
				title: "Single-language story",
				body: "Rust runtime + TypeScript functions, everywhere. Nakama is Go (server) + your choice (Go / Lua / TS handlers) + SDK language. Polyglot is a feature for some teams and a complexity tax for others.",
			},
			{
				title: "Fully OSS — no enterprise tier dark-pattern",
				body: "Pylon is MIT/Apache end-to-end. Nakama core is Apache 2.0 but Heroic Cloud + Satori analytics are proprietary paid services. Apples-to-apples self-hosted, fine; if you're evaluating the managed offering, the licensing shapes differ.",
			},
		],
		migration: [
			{ competitor: "Match handler (Lua / Go / TS)", pylon: "`SimState::tick` in Rust" },
			{ competitor: "Storage Engine objects", pylon: "Entities + policies" },
			{ competitor: "Leaderboards (built-in)", pylon: "Entity + sort query (build your own primitive)" },
			{ competitor: "Tournaments (built-in)", pylon: "Entity + scheduler (build your own primitive)" },
			{ competitor: "Streams pub/sub", pylon: "Pylon pub/sub + presence" },
			{ competitor: "Friends / groups (built-in)", pylon: "Custom entities" },
			{ competitor: "IAP receipt validation", pylon: "Pylon `action` calling Apple/Google APIs" },
			{ competitor: "Notifications (built-in)", pylon: "Custom entity + push provider integration" },
		],
		honestWeakness:
			"Nakama's feature set is genuinely deeper for game-specific concerns. If you need leaderboards-with-reset-schedules, tournaments-with-brackets, IAP validation, groups with rank hierarchies — those are real things you'd otherwise build yourself. Pylon's primitives let you build them, but Nakama hands them to you. For pure-game backends with deep social features, Nakama is often the right pick.",
		bothAnd:
			"Some studios run Nakama for the game backend and Pylon for the surrounding web/mobile app — account UI, billing, support tools, content management, marketing site. The two backends share user identity via OAuth or a shared SSO. Don't conflict, just different scopes.",
	},
];

export function getComparison(slug: string): Comparison | undefined {
	return COMPARISONS.find((c) => c.slug === slug);
}

export function comparisonSlugs(): string[] {
	return COMPARISONS.map((c) => c.slug);
}
