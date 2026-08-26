// Competitor comparison data for the /vs/* pages. Every Pylon-side capability
// claim here is checked against the framework source (crates/, packages/,
// apps/web/public/llms-full.txt) — concede only what's true, claim only what
// ships and is reachable from a TypeScript app. No "use both" sections: a /vs
// page makes the case for Pylon.

// Live since 2026-08-25. These pages were held back pre-launch to keep the
// site on what Pylon offers rather than competitor framing. Instant Cloud's
// shutdown changed the calculus: people are searching for a replacement
// backend now, and the InstantDB page plus the migration guide are the honest
// answer to that search. Every consumer — marketing-nav, site-footer, the /vs
// + /vs/[slug] routes, and the sitemap — respects this flag.
export const COMPARISONS_ENABLED = true;

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
	slug: string;
	competitor: string;
	competitorUrl: string;
	keyword: string;
	metaDescription: string;
	lede: string;
	tldr: {
		chooseCompetitor: string;
		choosePylon: string;
	};
	architecture: ComparisonTableRow[];
	sameShape: string[];
	competitorBetter: ComparisonItem[];
	pylonBetter: ComparisonItem[];
	migration: MigrationRow[] | null;
	honestWeakness: string;
};

export const COMPARISONS: Comparison[] = [
	{
		slug: "supabase",
		competitor: "Supabase",
		competitorUrl: "https://supabase.com",
		keyword: "Supabase alternative",
		metaDescription:
			"Open-source Supabase alternative. One Pylon binary provides auth, sync, file storage, row-level security, and native faceted search.",
		lede: "Supabase packages Postgres with GoTrue, PostgREST, Realtime, Storage, and Studio. Pylon provides most of that surface in one binary and can run on Postgres. The choice comes down to the trade-offs below.",
		tldr: {
			chooseCompetitor:
				"You want raw SQL as your primary interface (CTEs, materialized views, pgvector), you're comfortable operating multi-service deployments, or you need Postgres tools (Metabase, Hex, Retool) hitting your data directly.",
			choosePylon:
				"You want one binary on a VPS, native faceted search with live counts, in-process functions that share a transaction with your writes, or a typed entity API + realtime sync over the same data.",
		},
		architecture: [
			{
				dim: "Process count",
				pylon: "1",
				competitor:
					"7+ (Postgres, GoTrue, PostgREST, Realtime, Storage, Studio, Edge Functions)",
			},
			{
				dim: "Default DB",
				pylon: "SQLite (Postgres optional)",
				competitor: "Postgres (required)",
			},
			{
				dim: "Backed by",
				pylon: "Rust",
				competitor: "Postgres + Go + Elixir + Deno",
			},
			{
				dim: "Self-host",
				pylon: "scp + systemctl",
				competitor: "docker-compose with 7+ containers",
			},
			{
				dim: "Schema source of truth",
				pylon: "TypeScript (entity())",
				competitor: "SQL migrations",
			},
			{
				dim: "Faceted search",
				pylon: "Native (full-text + facets)",
				competitor: "tsvector for FTS, build facets yourself",
			},
			{
				dim: "Function-to-DB latency",
				pylon: "<1ms (same process)",
				competitor: "50–200ms (Edge → Postgres)",
			},
		],
		sameShape: [
			"Real-time subscriptions over WebSocket",
			"Built-in auth with magic links, password, and OAuth",
			"File storage with signed URLs",
			"Row-level access control (RLS for Supabase, policies for Pylon)",
			"Web + mobile SDKs",
			"Self-hostable, FOSS-licensed",
			"Managed cloud option",
		],
		competitorBetter: [
			{
				title: "Raw SQL as the interface",
				body: "Every Postgres feature is one query away: CTEs, window functions, JSONB operators, materialized views, foreign data wrappers, and partitioning. Pylon can run on Postgres, but exposes a typed entity API rather than arbitrary SQL.",
			},
			{
				title: "pgvector at million-scale",
				body: "pgvector's approximate indexes (HNSW, IVFFlat) handle million-row vector tables. Pylon ships built-in vector search (field.vector + ctx.db.vectorSearch) as exact k-NN — perfect recall and zero index maintenance to roughly 100k rows per entity; past that, pgvector's ANN wins.",
			},
			{
				title: "Larger ecosystem",
				body: "More libraries, more tutorials, more StackOverflow answers, more job postings. Years of battle-testing on PostgREST. Mature integrations with Metabase, Hex, Retool.",
			},
			{
				title: "Database UI for SQL exploration",
				body: "Supabase Studio is excellent for ad-hoc SQL. Pylon Studio focuses on entity inspection.",
			},
		],
		pylonBetter: [
			{
				title: "One binary instead of seven services",
				body: "Pylon uses one service, one port, and one config file. A comparable Supabase deployment uses docker-compose with Postgres, GoTrue, PostgREST, Realtime, Storage, Studio, and Edge Functions.",
			},
			{
				title: "Native faceted search",
				body: "Add search: to an entity and get full-text hits and live facet counts in one call. Supabase has tsvector for full-text search; facets require custom queries.",
			},
			{
				title: "Vector search with zero setup",
				body: "embedding: field.vector(1536) is one schema line; ctx.llm.embed and ctx.db.vectorSearch are built in. No extension to enable, no index DDL, no separate vector database until you genuinely outgrow exact search.",
			},
			{
				title: "Functions share a transaction with writes",
				body: "A Pylon mutation runs in-process with ctx.db and rolls back when it throws. Supabase Edge Functions are separate processes that reach Postgres over HTTP, so atomicity needs an explicit BEGIN/COMMIT.",
			},
			{
				title: "Realtime that reconciles optimistic writes",
				body: "Supabase Realtime streams raw row changes (Postgres Changes) and leaves optimistic-write reconciliation to your client. Pylon's sync engine pushes server-computed diffs and reconciles your optimistic writes for you.",
			},
			{
				title: "Native React SSR in the same binary",
				body: "Server-render your frontend from the process that holds your data. Supabase pairs with a separate Next.js host and an API hop between origins.",
			},
		],
		migration: [
			{
				competitor: "SQL schema + RLS",
				pylon: "entity() + policy() in TypeScript",
			},
			{
				competitor: "supabase.from('todos').select()",
				pylon: 'db.useQuery("Todo")',
			},
			{
				competitor: "supabase.auth.signInWithOtp(...)",
				pylon: "magic-link auth, built in",
			},
			{
				competitor: "Storage buckets",
				pylon: "presigned uploads (S3 / R2 / local)",
			},
			{
				competitor: "Edge Functions",
				pylon: "mutation / action in functions/*.ts",
			},
			{
				competitor: "Realtime subscriptions",
				pylon: "db.useQuery (server-authoritative)",
			},
			{
				competitor: "auth.users",
				pylon: "Pylon's User entity (you control the shape)",
			},
		],
		honestWeakness:
			"Supabase exposes raw SQL as the primary interface, with direct SQL-tool interop (Metabase, Hex, Mode, Retool) and the full Postgres feature set one query away. Pylon can use the same Postgres, but reaches data through a typed entity API. Supabase is a better fit today for ad-hoc analytics and the wider SQL ecosystem.",
	},
	{
		slug: "convex",
		competitor: "Convex",
		competitorUrl: "https://convex.dev",
		keyword: "Convex alternative",
		metaDescription:
			"Open-source Convex alternative. Pylon packages TypeScript-first reactive queries, native search, and SSR in one permissively licensed binary you can self-host on a small VPS.",
		lede: "Pylon and Convex are the two TypeScript-first reactive backends. Both ship reactive queries, schema-as-code, and a managed cloud. The honest differences are licensing, deployment shape, and what's in the box.",
		tldr: {
			chooseCompetitor:
				"You want the most polished pure-TS dev loop, you'll stay on Convex's cloud (or accept their FSL-licensed self-host), and you don't need native SSR or faceted search.",
			choosePylon:
				"You want a single-binary self-host, a permissive license, native faceted search, or server-rendered React in the same process.",
		},
		architecture: [
			{
				dim: "Process model",
				pylon: "One service (Rust + Bun)",
				competitor: "Hosted service / multi-service self-host",
			},
			{
				dim: "Default store",
				pylon: "SQLite (Postgres optional)",
				competitor: "Custom Convex DB",
			},
			{
				dim: "License",
				pylon: "MIT OR Apache-2.0",
				competitor: "FSL — converts to Apache after 2 yrs",
			},
			{
				dim: "Self-host on day 1",
				pylon: "Yes — one binary",
				competitor: "Yes — multi-service",
			},
			{
				dim: "Faceted search",
				pylon: "Built-in (full-text + facets)",
				competitor: "Roll your own queries",
			},
			{
				dim: "Native SSR",
				pylon: "Yes — React in the same binary",
				competitor: "Pair with Next.js",
			},
			{
				dim: "Native mobile sync",
				pylon: "Swift engine at parity",
				competitor: "React Native",
			},
		],
		sameShape: [
			"Reactive queries that auto-update the UI on writes",
			"TypeScript-first query / mutation / action server functions",
			"Schema as code with entities and types end to end",
			"Real-time WebSocket sync, optimistic mutations",
			"Built-in auth (magic links, OAuth), file storage",
			"React and React Native SDKs",
			"Self-host + managed cloud options",
		],
		competitorBetter: [
			{
				title: "Pure-TS dev loop polish",
				body: "Convex has invested heavily in DX polish. Type inference flows end-to-end without a codegen step. Pylon's TS flow is tight, but Convex's is a touch tighter out of the box.",
			},
			{
				title: "Larger team + ecosystem",
				body: "Well-funded company, more docs, more examples, more StackOverflow answers, more job postings.",
			},
			{
				title: "Vector search at million-scale",
				body: "Convex's vector indexes are approximate and built for large tables. Pylon's built-in vector search (field.vector + ctx.llm.embed + ctx.db.vectorSearch) is exact k-NN — perfect recall to roughly 100k rows per entity; past that, Convex's ANN wins.",
			},
		],
		pylonBetter: [
			{
				title: "One binary to self-host",
				body: "Install the binary and Bun on a VPS, or run `pylon deploy`. Convex's self-host uses multiple services and a custom database.",
			},
			{
				title: "Native faceted search",
				body: "Add search: to an entity and get full-text hits and live facet counts in one call. Convex requires custom queries on top of full-text search.",
			},
			{
				title: "Server-rendered React in the box",
				body: "Pylon server-renders your frontend from the same binary, including file routing, <Link>, <Image>, metadata, and streaming. Convex pairs with a separate Next.js host.",
			},
			{
				title: "Permissive license",
				body: "MIT OR Apache-2.0. Convex's FSL bars you from running a competing managed Convex for two years. An edge case for most, but meaningful for devtools companies.",
			},
			{
				title: "A first-class Swift engine",
				body: "The full sync engine ported to Swift and kept at parity, for native iOS and Mac. Convex's mobile story is React Native.",
			},
		],
		migration: [
			{
				competitor: "defineSchema(...)",
				pylon: "buildManifest({ entities: [...] })",
			},
			{
				competitor: "query / mutation / action",
				pylon: "Same names, identical mental model",
			},
			{ competitor: "useQuery(api.tasks.list)", pylon: 'db.useQuery("Task")' },
			{
				competitor: 'ctx.db.insert("tasks", {...})',
				pylon: 'ctx.db.insert("Task", {...})',
			},
			{ competitor: "Convex auth", pylon: "Magic-link / OAuth / OIDC" },
			{
				competitor: "Convex file storage",
				pylon: "presigned uploads (S3 / R2 / local)",
			},
			{
				competitor: "Convex scheduled functions",
				pylon: "ctx.scheduler + durable ctx.workflows",
			},
			{ competitor: "Convex search index", pylon: "Per-entity search config" },
			{
				competitor: "Convex vector search",
				pylon: "field.vector + ctx.db.vectorSearch",
			},
		],
		honestWeakness:
			"Convex has more developer mindshare today and more polish in reactive query batching, type-inference depth, and IDE integration. If you want the most polished pure-TS reactive backend and do not need single-process self-hosting, faceted search, or native SSR, Convex is a strong choice.",
	},
	{
		slug: "firebase",
		competitor: "Firebase",
		competitorUrl: "https://firebase.google.com",
		keyword: "Firebase alternative",
		metaDescription:
			"Open-source Firebase alternative. Pylon ships realtime sync, auth, functions, file storage, and faceted search in one self-hostable binary with in-process functions.",
		lede: "Firebase combines Firestore, Realtime Database, Auth, Cloud Functions, and Storage for mobile apps. Pylon covers much of that surface and adds declarative schema, faceted search, and native SSR.",
		tldr: {
			chooseCompetitor:
				"You're building a mobile-first app, you want Google's ecosystem (FCM push, Crashlytics, GA4) tightly integrated, and you accept a closed-source backend.",
			choosePylon:
				"You want self-host, a permissive license, declarative schema instead of schemaless documents, faceted search without Algolia, or no cold starts on functions.",
		},
		architecture: [
			{
				dim: "License",
				pylon: "MIT OR Apache-2.0",
				competitor: "Closed source",
			},
			{
				dim: "Self-hostable",
				pylon: "Yes, on any Linux box",
				competitor: "No, Google-only",
			},
			{
				dim: "Schema",
				pylon: "Declarative (TypeScript)",
				competitor: "Schemaless (Firestore documents)",
			},
			{
				dim: "Functions runtime",
				pylon: "In-process",
				competitor: "Cloud Functions (separate)",
			},
			{
				dim: "Function cold start",
				pylon: "None",
				competitor: "1–10 seconds for cold containers",
			},
			{
				dim: "Full-text search",
				pylon: "Built-in (full-text + facets)",
				competitor: "Mirror to Algolia / Typesense",
			},
			{ dim: "Open source", pylon: "Yes", competitor: "No" },
		],
		sameShape: [
			"Real-time sync over WebSocket",
			"Built-in auth (email, OAuth, anonymous)",
			"Functions for server-side logic",
			"File storage with signed URLs",
			"Web + mobile + native SDKs",
			"Managed cloud (Firebase / Smallware)",
		],
		competitorBetter: [
			{
				title: "Google ecosystem integrations",
				body: "FCM push, Crashlytics, GA4, Remote Config, A/B testing, in-app messaging. Mobile-app-shaped concerns are first-class. With Pylon you'd integrate FCM + a crash reporter + your analytics provider on top.",
			},
			{
				title: "Mobile-first SDKs maturity",
				body: "Firebase's iOS, Android, and Unity SDKs are deeply integrated with platform features (push tokens, app-startup hooks, offline persistence) and have years of polish.",
			},
			{
				title: "Transparent horizontal scale",
				body: "Firestore is built to shard transparently. Pylon's SQLite default is single-process, and Postgres mode eventually reaches its own limits. Globe-scale apps need a distributed database such as Spanner or DynamoDB.",
			},
		],
		pylonBetter: [
			{
				title: "Declarative schema",
				body: "Firestore lets every document have its own shape. That helps with prototyping, but mistyped fields become orphan data and refactors require manual sweeps. Pylon's entity() definition is the single source of truth.",
			},
			{
				title: "No cold starts",
				body: "Pylon functions run in a warm, in-process runtime. Firebase Cloud Functions can add a 1–10 second delay when a container starts cold.",
			},
			{
				title: "Functions share a transaction with writes",
				body: "ctx.db inside a mutation is a transaction. Firebase Cloud Functions reach Firestore through the Admin SDK from a separate process, so the function and database write do not share one transaction.",
			},
			{
				title: "Full-text + faceted search built-in",
				body: "Pylon includes full-text search and facets in the binary. Firebase recommends integrating a separate search service such as Algolia.",
			},
			{
				title: "Self-host on day one",
				body: "Pylon runs as one binary on a Linux box you control. Firebase runs only on Google's infrastructure.",
			},
			{
				title: "Predictable pricing",
				body: "Firebase prices reads, writes, deletes, egress, invocations, and GB-seconds separately. Smallware lists one price per dimension; self-hosted users pay their infrastructure bill.",
			},
		],
		migration: [
			{
				competitor: "Firestore collections",
				pylon: "entity() definitions in TypeScript",
			},
			{
				competitor: "Security rules (custom DSL)",
				pylon: "policy() with boolean expressions",
			},
			{
				competitor: "firestore.collection().onSnapshot()",
				pylon: 'db.useQuery("Entity")',
			},
			{
				competitor: "Cloud Functions (HTTP-triggered)",
				pylon: "mutation / action in functions/*.ts",
			},
			{
				competitor: "Firebase Auth",
				pylon: "Magic-link / OAuth (export users, import as User rows)",
			},
			{
				competitor: "Cloud Storage",
				pylon: "presigned uploads (S3 / R2 / local)",
			},
			{
				competitor: "Firebase Cloud Messaging",
				pylon:
					"Keep FCM; register tokens via an action and send from your function",
			},
		],
		honestWeakness:
			"Firebase's mobile-first integrations include FCM, Crashlytics, A/B testing through Remote Config, and in-app messaging. Pylon does not provide equivalents. Firebase has more polish for apps centered on push notifications and mobile experimentation. With Pylon, bring your own analytics, crash reporting, and push provider; register FCM through an action.",
	},
	{
		slug: "instantdb",
		competitor: "InstantDB",
		competitorUrl: "https://instantdb.com",
		keyword: "InstantDB alternative",
		metaDescription:
			"Open-source InstantDB alternative. Pylon combines optimistic local-first sync, server functions, row-level policies, native SSR, and self-hosting in one binary.",
		lede: "Instant Cloud is shutting down. Signups are closed, hosted apps stop on August 31, 2027, and the team has joined OpenAI — Instant stays open source but unmaintained, so your options are self-hosting a Clojure/JVM stack or moving. InstantDB and Pylon both center on local-first, optimistic sync; Pylon adds server functions, policies, SSR, and a managed cloud in one binary. Step-by-step move: docs.pylonsync.com/migrate/instantdb.",
		tldr: {
			chooseCompetitor:
				"You want the purest client-side local-first DX, you like the InstaQL graph query language, your backend logic is light enough to live in permission rules and the client, and you have the operations appetite to self-host an unmaintained JVM + Postgres 16 stack now that the hosted option is ending.",
			choosePylon:
				"You want server functions running in-process with your data, deny-by-default row-level policies, native React SSR, faceted search, or to self-host the whole thing as a single binary.",
		},
		architecture: [
			{
				dim: "Shape",
				pylon: "Full-stack framework",
				competitor: "Client-leaning realtime DB",
			},
			{
				dim: "Server logic",
				pylon: "In-process query / mutation / action",
				competitor: "Rules + client; lighter server surface",
			},
			{
				dim: "Schema",
				pylon: "TypeScript entity()",
				competitor: "Typed schema + InstaQL graph queries",
			},
			{
				dim: "Access control",
				pylon: "Row-level policies, deny-by-default",
				competitor: "Permission rules",
			},
			{
				dim: "Native SSR",
				pylon: "Yes — React in the same binary",
				competitor: "Client-first; pair a renderer",
			},
			{
				dim: "Self-host",
				pylon: "One binary on any Linux box",
				competitor: "Clojure/JVM + Postgres 16, unmaintained",
			},
			{
				dim: "Search",
				pylon: "Full-text + facets built-in",
				competitor: "Query the graph yourself",
			},
		],
		sameShape: [
			"Local-first, optimistic writes that reconcile in the background",
			"Instant reactive queries over a live connection",
			"Typed schema as code",
			"Built-in auth and access rules",
			"Offline-capable clients",
			"React SDK",
			"Managed cloud option (Instant's ends August 31, 2027)",
		],
		competitorBetter: [
			{
				title: "InstaQL graph queries",
				body: "InstantDB's query language fetches deeply nested relational data in one declarative shape. Pylon has nested-relation reads via queryGraph, but InstaQL's graph DSL is more expressive for deeply nested, graph-shaped reads.",
			},
			{
				title: "Client-first simplicity",
				body: "For apps whose logic lives on the client, InstantDB keeps setup small by putting schema, permissions, and queries mostly in the frontend.",
			},
			{
				title: "Focused surface area",
				body: "InstantDB focuses on relational sync. Its smaller surface is easier to learn than a full framework when that is all you need.",
			},
		],
		pylonBetter: [
			{
				title: "Real server functions",
				body: "Pylon runs query / mutation / action in-process with ctx.db, ctx.auth, and validators. Payments, third-party calls, and privileged writes have a server-side home that shares a transaction with your data.",
			},
			{
				title: "Row-level policies, deny-by-default",
				body: "Row-level access rules live next to the schema and run on the hot path of every read and write, including sync subscriptions. Unguarded entities are default-denied; a linter flags them at dev startup.",
			},
			{
				title: "Native React SSR",
				body: "Server-render your frontend from the same binary with file routing, <Link>, <Image>, metadata, streaming, and ISR. InstantDB is client-first and leaves rendering to you.",
			},
			{
				title: "Faceted search in the box",
				body: "Add search: to an entity for full-text hits + live facet counts, and field.vector for built-in vector search — same binary. Deferred work runs through ctx.scheduler; multi-step durable workflows through ctx.workflows.",
			},
			{
				title: "Self-host as one binary",
				body: "Run the whole stack on a VPS you control, or on Smallware. Instant's own self-host path is a Clojure/JVM server on Postgres 16 plus a reverse proxy and an email provider, and its cloud stops on August 31, 2027.",
			},
			{
				title: "A first-class Swift engine",
				body: "The full sync engine runs natively in Swift and stays at parity with the web client on iOS and Mac.",
			},
		],
		migration: [
			{
				competitor: "InstantDB schema",
				pylon: "entity() definitions in TypeScript",
			},
			{
				competitor: "Permission rules",
				pylon: "policy() with boolean expressions",
			},
			{
				competitor: "useQuery / InstaQL",
				pylon: 'db.useQuery("Entity", { where })',
			},
			{
				competitor: "transact / tx",
				pylon: "mutation / action in functions/*.ts",
			},
			{ competitor: "InstantDB auth", pylon: "Magic-link / OAuth / OIDC" },
			{
				competitor: "Client-side logic",
				pylon: "Move privileged logic into server functions",
			},
		],
		honestWeakness:
			"For client-only apps that need InstaQL graph queries and frontend-defined permissions, InstantDB is leaner to adopt and more expressive for nested relational reads. Pylon adds surface area when you do not need server functions, SSR, or self-hosting.",
	},
];

export function comparisonSlugs(): string[] {
	return COMPARISONS.map((c) => c.slug);
}

export function getComparison(slug: string): Comparison | undefined {
	return COMPARISONS.find((c) => c.slug === slug);
}
