// Content source of truth for the /product/* pages. One entry per primitive,
// keyed by the slug in lib/site-nav.ts. The dynamic route at
// app/product/[slug]/page.tsx renders these through a shared template, so
// adding a primitive = adding an entry here (+ a nav link).
//
// Code samples use the real Pylon API (entity/field/query/mutation/v/policy
// from @pylonsync/sdk + @pylonsync/functions, db.useQuery from
// @pylonsync/react) — the same shapes the create-pylon templates ship.

export interface ProductSection {
	title: string;
	body: string;
}

export interface ProductCode {
	/** Caption above the code block (filename or context). */
	label: string;
	code: string;
}

export interface ProductContent {
	slug: string;
	/** Short label for index cards + breadcrumbs. */
	navLabel: string;
	/** Eyebrow above the H1. */
	category: string;
	/** H1 — punchy, benefit-led. */
	title: string;
	/** Lede paragraph under the H1. */
	tagline: string;
	/** 3–4 quick wins, shown as a checklist. */
	highlights: string[];
	/**
	 * What a team normally assembles by hand to get this one capability. Framed
	 * as the alternative you're comparing against, not a claim about Pylon's
	 * internals — keep every line something you'd actually have to build.
	 */
	replaces: string[];
	/** Optional hero code sample. */
	code?: ProductCode;
	/** Deep-dive blocks. */
	sections: ProductSection[];
	/** SEO. */
	metaTitle: string;
	metaDescription: string;
	/** Related primitive slugs (cross-links at the bottom). */
	related: string[];
}

export const PRODUCTS: Record<string, ProductContent> = {
	sync: {
		slug: "sync",
		navLabel: "Sync engine",
		category: "Sync engine",
		title: "Live queries over WebSocket.",
		tagline:
			"db.useQuery is a WebSocket subscription. Pylon walks the change log on every write and pushes the exact diff to each subscribed client, keeping the UI current without polling or cache invalidation.",
		highlights: [
			"Subscriptions update in milliseconds because the server pushes diffs instead of waiting for clients to poll",
			"Local-first: reads hit an in-browser store, writes apply optimistically and reconcile",
			"Queue mutations offline and sync them when the connection returns",
			"Multi-tab coherent via a leader election over BroadcastChannel",
		],
		replaces: [
			"A WebSocket server, plus reconnect and backoff logic",
			"A client cache and the invalidation rules that keep it honest",
			"Optimistic update plumbing and its rollback path",
			"A polling loop, or an SSE fallback for browsers that drop the socket",
		],
		code: {
			label: "components/Messages.tsx",
			code: `import { db } from "@pylonsync/react";

function Messages({ roomId }: { roomId: string }) {
  // A live subscription. Re-renders the instant anyone,
  // anywhere, inserts or edits a matching Message row.
  const { data: messages } = db.useQuery<Message>("Message", {
    where: { roomId },
  });

  return messages.map((m) => <Bubble key={m.id} message={m} />);
}`,
		},
		sections: [
			{
				title: "The server computes the diff",
				body: "On every write, Pylon walks an append-only change log and finds the affected subscriptions. Clients receive a minimal insert, update, or tombstone instead of fetching the result again. A 10,000-row table therefore updates as cheaply as a 10-row one.",
			},
			{
				title: "Optimistic by default, consistent always",
				body: "Mutations apply locally before the round-trip so the UI never waits on the network. The engine tracks each pending op, reconciles against the authoritative server state when the ack lands, and rolls back cleanly if a policy rejects the write. One engine handles the race conditions that otherwise spread across optimistic UI code.",
			},
			{
				title: "One engine, every client",
				body: "The same sync protocol drives the TypeScript engine in the browser and the Swift engine on iOS and Mac. Both speak the same wire format, hold the same guarantees, and stay at feature parity. Your web and native apps see the same data the same way.",
			},
		],
		metaTitle: "Pylon Sync Engine: live queries over WebSocket, local-first",
		metaDescription:
			"db.useQuery is a live WebSocket subscription. Pylon pushes the exact diff on every write, supports offline work, and keeps tabs coherent without polling or cache invalidation.",
		related: ["database", "realtime", "functions"],
	},

	database: {
		slug: "database",
		navLabel: "Database",
		category: "Database",
		title: "Typed schema. Migrations on save.",
		tagline:
			"Declare entities in TypeScript with field.string / int / datetime / id and composite indexes. Pylon diffs the schema and applies migrations automatically. SQLite is the one-file default. Point DATABASE_URL at Postgres and the same schema follows.",
		highlights: [
			"One fully typed schema file gives your editor every field",
			"Migrations apply on save in dev; no migration files to hand-author",
			"SQLite is the default: a single file, nothing to provision",
			"Set DATABASE_URL=postgres://… and the same schema targets Postgres",
		],
		replaces: [
			"A migration tool and the version table it owns",
			"An ORM layer plus whatever generates its types",
			"A hand-written REST or GraphQL API over your own tables",
			"Connection wiring between the API service and the database",
		],
		code: {
			label: "schema.ts",
			code: `import { entity, field } from "@pylonsync/sdk";

const Room = entity("Room", {
  slug: field.string(),
  name: field.string(),
  createdAt: field.datetime(),
});

const Message = entity("Message", {
  roomId: field.id("Room").readonly(),
  authorId: field.id("User").readonly(),
  body: field.string(),
  createdAt: field.datetime().readonly(),
});`,
		},
		sections: [
			{
				title: "Field types that mean something",
				body: "field.id(\"Room\") is a typed foreign key, field.richtext() carries formatting, field.datetime() is timezone-correct, field.encrypted() is sealed at rest. Add .readonly() to block HTTP PATCH from rewriting identity fields, .index() for composite indexes. The types flow all the way to db.useQuery on the client.",
			},
			{
				title: "SQLite to Postgres without a rewrite",
				body: "Prototype on SQLite. It is a file, ships in the binary, and needs no setup. When you need Postgres, bring your own connection string or use managed Postgres on Cloud (private beta), then change one environment variable. The schema, queries, policies, and sync engine stay the same; only the storage target changes.",
			},
			{
				title: "Migrations you don't babysit",
				body: "Change a field, save the file, and Pylon diffs the live schema against your declaration and applies the migration. In production the same diff runs as a guarded step on deploy. No migrations/ folder full of timestamped SQL to keep in order.",
			},
		],
		metaTitle: "Pylon Database: typed schema, auto migrations, SQLite or Postgres",
		metaDescription:
			"Declare entities in TypeScript. Migrations apply on save. SQLite by default with zero setup; point DATABASE_URL at Postgres and the same schema follows.",
		related: ["sync", "auth", "search"],
	},

	auth: {
		slug: "auth",
		navLabel: "Auth & policies",
		category: "Auth & access control",
		title: "Auth and access rules, built in.",
		tagline:
			"Magic-link email, 25+ OAuth providers, generic OIDC for any IdP, guest sessions, and API keys are built in. Policy expressions live next to your schema and gate every row in the hot path.",
		highlights: [
			"Magic-link, Google / GitHub / Apple / Microsoft / Discord / Slack and 20+ more",
			"Generic OIDC discovery for any enterprise IdP; SAML SSO on Cloud",
			"Guest sessions and API keys for public apps and machine clients",
			"Row-level policies compile to bytecode and run on every read and write",
		],
		replaces: [
			"A hosted auth provider and its SDK on every client",
			"Session storage, refresh, and revocation handling",
			"Route middleware that re-checks permissions on each endpoint",
			"A second rules language for row-level access",
		],
		code: {
			label: "schema.ts — a policy",
			code: `import { policy } from "@pylonsync/sdk";

const messagePolicy = policy({
  name: "message_access",
  entity: "Message",
  allowRead: "true",
  allowInsert: "auth.userId != null and data.authorId == auth.userId",
  allowUpdate: "false",
  allowDelete: "data.authorId == auth.userId",
});`,
		},
		sections: [
			{
				title: "Every provider, one config",
				body: "Turn on magic-link and a wall of OAuth providers from the same auth() block. Generic OIDC discovery means any IdP that publishes a discovery document works without a bespoke integration. Guest sessions let anonymous users interact before they sign up; API keys authenticate scripts and machine clients.",
			},
			{
				title: "Policies that can't be bypassed",
				body: "Access rules such as auth.userId == data.authorId and auth.tenantId == data.orgId live next to the entity they protect. Pylon enforces them on every read and write, including sync subscriptions. The policy linter flags entities without a policy at dev startup, and unguarded entities default to deny.",
			},
			{
				title: "Multi-tenant without the footguns",
				body: "Sessions carry an active tenant. Scope reads with auth.tenantId == data.orgId and the engine filters every subscription, list route, and export by tenant automatically. The tenant check lives in one policy instead of being reimplemented across endpoints.",
			},
		],
		metaTitle: "Pylon Auth: magic-link, OAuth, OIDC, and row-level policies",
		metaDescription:
			"Magic-link, 25+ OAuth providers, OIDC, guest sessions, and API keys are built in. Row-level policy expressions live next to your schema and run on every read and write.",
		related: ["database", "functions", "sync"],
	},

	functions: {
		slug: "functions",
		navLabel: "Server functions",
		category: "Server functions",
		title: "Backend logic in TypeScript.",
		tagline:
			"Write queries, mutations, and actions with v.* validators. The filename is the RPC name. Call them from React with a typed client, and writes broadcast through the sync engine automatically.",
		highlights: [
			"query / mutation / action with validated, typed arguments",
			"The filename is the RPC name: functions/sendMessage.ts is callable as sendMessage",
			"ctx.db honors policies; ctx.auth carries the caller; ctx.error throws typed failures",
			"Writes emit change events, so live queries update with zero extra wiring",
			"Test logic and components with `pylon test`, Bun, and @testing-library",
		],
		replaces: [
			"An RPC or REST layer and the client that calls it",
			"Request validation written out per endpoint",
			"A generated API client you re-sync after every change",
			"A separate serverless target to deploy backend logic to",
		],
		code: {
			label: "functions/sendMessage.ts",
			code: `import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: { roomId: v.id("Room"), body: v.string() },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const id = await ctx.db.insert("Message", {
      roomId: args.roomId,
      authorId: ctx.auth.userId,
      body: args.body.trim(),
      createdAt: new Date().toISOString(),
    });
    return ctx.db.get("Message", id);   // broadcasts to subscribers
  },
});`,
		},
		sections: [
			{
				title: "Validated arguments, end to end",
				body: "v.string(), v.id(\"Room\"), v.object({…}) describe the shape once. Pylon validates inbound calls and infers the handler's argument types, turning a mistyped field into a local type error instead of a production 500.",
			},
			{
				title: "The context object is the whole backend",
				body: "ctx.db reads and writes through the policy engine using the caller's identity. ctx.auth carries the user and tenant. ctx.error throws structured errors the client can switch on. ctx.schedule enqueues background work; ctx.llm calls a model. The ORM, the job queue, and the auth layer are already part of the context.",
			},
			{
				title: "Writes are live by default",
				body: "Every mutation and action that writes emits typed change events. Any db.useQuery watching affected rows updates instantly. The function returns the row; Pylon handles channels, cache invalidation, and refetching.",
			},
			{
				title: "Test it without standing anything up",
				body: "Keep gating, pricing, and validation decisions in plain functions and test them with `pylon test`, Bun’s runner against an in-memory Pylon. Component tests render with @testing-library + happy-dom, wired into every new app by default. You do not need fixtures, a test database, or framework mocks.",
			},
		],
		metaTitle: "Pylon Server Functions: typed queries, mutations, and actions",
		metaDescription:
			"Write backend logic in TypeScript with v.* validators. Filename is the RPC name. ctx.db honors policies, writes broadcast to live queries automatically.",
		related: ["database", "auth", "workflows"],
	},

	realtime: {
		slug: "realtime",
		navLabel: "Realtime",
		category: "Realtime & multiplayer",
		title: "Rooms, presence, and authoritative shards.",
		tagline:
			"WebSocket rooms provide per-member presence and broadcast for chat and collaboration. Authoritative Rust shards run multiplayer games at 20, 30, or 60 ticks per second on the same server as the rest of your app.",
		highlights: [
			"Rooms with join/leave events, per-member presence, and broadcast",
			"Authoritative 20/30/60 tps simulation loops written in Rust",
			"Area-of-interest filtering, snapshot + delta replication, late-join, observer slots",
			"Same binary, same auth, same data model as the rest of your app",
		],
		replaces: [
			"A presence service and its heartbeat and timeout rules",
			"A pub/sub broker for ephemeral state that must not be persisted",
			"Cursor, typing, and who's-online broadcast plumbing",
			"A second socket running alongside your data connection",
		],
		code: {
			label: "components/Presence.tsx",
			code: `import { useRoom } from "@pylonsync/react";

function Whiteboard({ boardId }: { boardId: string }) {
  const { members, broadcast, presence } = useRoom("board:" + boardId, {
    presence: { cursor: { x: 0, y: 0 } },
  });

  // members[].presence.cursor updates live as others move.
  return <Cursors members={members} onMove={(p) => presence.set({ cursor: p })} />;
}`,
		},
		sections: [
			{
				title: "Presence and broadcast for collaboration",
				body: "Join a room, publish a cursor, selection, or typing indicator, and receive everyone else's presence in real time. Broadcast ephemeral events that do not need to hit the database. The same server that holds your data supplies the multiplayer layer for docs, boards, and dashboards.",
			},
			{
				title: "Authoritative simulation for games",
				body: "For real-time games, run a fixed-tick loop in Rust where the server is the source of truth. Area-of-interest filtering sends each player only what's near them; snapshot-plus-delta replication keeps bandwidth flat; late-joiners get a full snapshot then deltas. Observer slots let spectators watch without affecting the sim.",
			},
			{
				title: "One stack for app and game",
				body: "The app primitives (schema, auth, sync) and game primitives (rooms, shards) run in the same binary. A multiplayer game's accounts, inventory, matchmaking, and tick loop share one typed database.",
			},
		],
		metaTitle: "Pylon Realtime: rooms, presence, and authoritative game shards",
		metaDescription:
			"WebSocket rooms with presence and broadcast for collaboration, plus authoritative 20/30/60 tps Rust shards with area-of-interest filtering for multiplayer games.",
		related: ["sync", "functions", "swift"],
	},

	storage: {
		slug: "storage",
		navLabel: "Storage",
		category: "File storage",
		title: "Presigned uploads, built in.",
		tagline:
			"Get presigned upload URLs without standing up an upload service. Files land on local disk in development and an S3-compatible bucket such as R2, Backblaze, MinIO, or AWS in production. One environment variable selects the backend.",
		highlights: [
			"Presigned uploads send files straight from the client to storage",
			"Local disk in dev, S3-compatible buckets in prod",
			"R2, Backblaze B2, MinIO, or AWS S3 via one env var",
			"File metadata lives in your typed schema, gated by the same policies",
		],
		replaces: [
			"An object-store SDK and a service to sign upload URLs",
			"An upload proxy that streams file bytes through your API",
			"Access checks bolted on after the fact to file URLs",
			"Per-environment bucket policies kept in sync by hand",
		],
		code: {
			label: "components/Avatar.tsx",
			code: `import { FileUpload } from "@pylonsync/react";

function AvatarPicker() {
  return (
    <FileUpload
      accept="image/*"
      onUploaded={(file) => callFn("setAvatar", { fileId: file.id })}
    />
  );
}`,
		},
		sections: [
			{
				title: "Direct-to-storage, signed",
				body: "Pylon gives the client a short-lived presigned URL, so the bytes go straight to the bucket and large files do not tie up a server worker. The same access policies that guard your rows also guard downloads.",
			},
			{
				title: "The same code, any backend",
				body: "Develop against the local filesystem with nothing to configure. In production, point one variable at R2, Backblaze, MinIO, or S3. The storage driver changes underneath the same upload API, so your code does not.",
			},
			{
				title: "Files are first-class rows",
				body: "An uploaded file is a record in your schema with an owner, content type, and size. You can query it, gate it with a policy, and sync it like any other row. Owner-scoping on download prevents users from retrieving another user's file by guessing its id.",
			},
		],
		metaTitle: "Pylon Storage: presigned uploads to local disk or any S3 bucket",
		metaDescription:
			"Built-in presigned file uploads use local disk in development or an S3-compatible bucket in production. One environment variable selects R2, Backblaze, MinIO, or S3. Files are policy-gated rows.",
		related: ["database", "auth", "functions"],
	},

	search: {
		slug: "search",
		navLabel: "Search",
		category: "Full-text search",
		title: "Full-text search, in your database.",
		tagline:
			"Add search: to an entity and get BM25 ranking, live facets, and sort across millions of rows. The index lives in the same database and is maintained in the same transaction as your writes, so results are always consistent with your data.",
		highlights: [
			"Built-in BM25 relevance ranking",
			"Live facets and sort keep filter counts current as data changes",
			"Index maintained transactionally with your writes; never stale",
			"Search scales with your database, with no separate service to deploy or sync",
		],
		replaces: [
			"A search cluster to provision, monitor, and pay for",
			"An indexing pipeline, and the lag between write and searchable",
			"Reindex jobs to run after every schema change",
			"Facet counts recomputed in application code on every query",
		],
		code: {
			label: "schema.ts — searchable entity",
			code: `const Product = entity("Product", {
  name: field.string(),
  description: field.string(),
  category: field.string().index(),
  price: field.int(),
}).search({
  fields: ["name", "description"],
  facets: ["category"],
});`,
		},
		sections: [
			{
				title: "Relevance, in your database",
				body: "Pylon keeps a BM25 index in the same database and updates it in the same transaction as every write. Search results stay consistent with your data, and you operate one system instead of two.",
			},
			{
				title: "Facets and sort, live",
				body: "Declare facet fields and Pylon maintains their counts as rows change. Build product-search filters for category, price, or status and watch the counts update as inventory moves.",
			},
			{
				title: "Search scales with your database",
				body: "Because search is part of the database, it scales with the storage you already run and uses the same infrastructure and backups.",
			},
		],
		metaTitle: "Pylon Search: BM25 full-text and facets in your database",
		metaDescription:
			"Add search: to an entity for BM25 ranking, live facets, and sort across millions of rows. The index lives in your DB, maintained transactionally, always consistent with your data.",
		related: ["database", "sync", "functions"],
	},

	workflows: {
		slug: "workflows",
		navLabel: "Workflows & jobs",
		category: "Workflows, jobs & cron",
		title: "Durable workflows and background jobs.",
		tagline:
			"Long-running workflows can sleep, retry, and wait for events across restarts because Pylon checkpoints every step. Enqueue background jobs with ctx.schedule and version cron entries in the manifest with your code.",
		highlights: [
			"Durable workflows sleep for days, wait for events, and retry across deploys and crashes",
			"State checkpointed to storage on every step; resume exactly where they left off",
			"Background jobs via ctx.schedule run work after the response is sent",
			"Cron lives in the manifest, so the schedule is reviewed and versioned like code",
		],
		replaces: [
			"A queue broker and the worker fleet that drains it",
			"A cron host living separately from the app it triggers",
			"Retry, backoff, and idempotency logic written by hand",
			"A dead-letter store that someone has to remember to check",
		],
		code: {
			label: "functions/onSignup.ts",
			code: `import { workflow } from "@pylonsync/functions";

export default workflow({
  async handler(ctx, { userId }) {
    await ctx.step("welcome", () => sendEmail(userId, "welcome"));
    await ctx.sleep("3d");
    const active = await ctx.step("check", () => isActive(userId));
    if (!active) await ctx.step("nudge", () => sendEmail(userId, "comeback"));
  },
});`,
		},
		sections: [
			{
				title: "Workflows that outlive the process",
				body: "A workflow can sleep for three days, wait for a webhook, then continue after a deploy or crash. Pylon checkpoints each step, replays completed steps from their recorded results, and resumes at the next one. The workflow runs in the same runtime as your app.",
			},
			{
				title: "Jobs after the response",
				body: "Enqueue work with ctx.schedule to keep request latency low. Send an email, transcode an upload, or recompute a rollup after returning to the user. Jobs run on the same server with the same database and policy context.",
			},
			{
				title: "Cron as code",
				body: "Scheduled jobs are declared in the manifest, so the cron expression sits in your repo, goes through review, and ships atomically with the function it triggers. No clicking schedules into a dashboard that drifts from the code.",
			},
		],
		metaTitle: "Pylon Workflows: durable workflows, background jobs, and cron",
		metaDescription:
			"Durable multi-step workflows with sleep, retries, and event waits that survive restarts. Background jobs via ctx.schedule and version-controlled cron in the manifest.",
		related: ["functions", "database", "cloud"],
	},

	ssr: {
		slug: "ssr",
		navLabel: "SSR",
		category: "Server-side rendering",
		title: "Native React server rendering.",
		tagline:
			"Render your React frontend from the same server that runs your backend. The Pylon binary serves file-based routing, <Link> and <Image>, per-route metadata, Suspense streaming, and on-disk ISR caching.",
		highlights: [
			"Server-rendered React 19 with file-based routing under app/",
			"<Link> and <Image> follow familiar conventions and include an image optimizer",
			"Per-route metadata, streaming Suspense, error and not-found boundaries",
			"On-disk ISR cache keeps anonymous renders independent of the cloud app",
		],
		replaces: [
			"A separate rendering framework and its own build step",
			"A data-fetching layer written twice, once per environment",
			"Hydration mismatches to chase between server and client output",
			"A second deploy target for the frontend",
		],
		code: {
			label: "app/product/[slug]/page.tsx",
			code: `export async function generateMetadata({ params }) {
  const p = await getProduct(params.slug);
  return { title: p.name, description: p.summary };
}

export default function ProductPage({ params }) {
  return <ProductView slug={params.slug} />;
}`,
		},
		sections: [
			{
				title: "One server for frontend and backend",
				body: "The Pylon binary that holds your schema, runs your functions, and drives the sync engine also server-renders your React. The frontend and backend share one server, origin, deployment, and session on every request. This site uses that architecture.",
			},
			{
				title: "The conventions you already know",
				body: "app/ file routing with dynamic [slug] and catch-all [...slug] segments. <Link> for client navigation, <Image> with srcset and an optimizer. export const metadata or generateMetadata for SEO. loading.tsx for streaming fallbacks, error.tsx and not-found.tsx for boundaries.",
			},
			{
				title: "Cacheable by design",
				body: "Pylon verifies that anonymous renders are auth-independent, then caches them to disk (ISR) and the CDN. Public pages remain available when the app is busy. Authenticated renders resolve the session server-side, which prevents a logged-in flash.",
			},
		],
		metaTitle: "Pylon SSR: native React server rendering in one binary",
		metaDescription:
			"Server-render React from the same binary that runs your backend. File routing, <Link>/<Image>, per-route metadata, streaming Suspense, and on-disk ISR caching.",
		related: ["functions", "cloud", "studio"],
	},

	studio: {
		slug: "studio",
		navLabel: "Studio",
		category: "Admin studio",
		title: "An admin panel from your schema.",
		tagline:
			"Browse tables, inspect live queries, tail logs, and run ad-hoc mutations at /studio against any Pylon deployment. It's admin-gated in production and works the same in dev, staging, and prod.",
		highlights: [
			"Browse and edit every table with the schema as the UI",
			"Inspect live queries and the change log as writes happen",
			"Tail structured logs and run ad-hoc mutations",
			"Admin-gated in production; zero setup in dev",
		],
		replaces: [
			"A database GUI pointed at production credentials",
			"A log aggregator just to read local output",
			"Ad-hoc scripts to inspect which subscriptions are live",
			"An internal admin app built to make one-off edits",
		],
		sections: [
			{
				title: "Your schema is the interface",
				body: "Studio reads your entity definitions and renders the right editor for each field: date pickers for datetimes, references for ids, and rich-text editors for richtext. It tracks your schema without a separate admin framework or model registration.",
			},
			{
				title: "See the system working",
				body: "Watch the change log stream as mutations land, inspect which subscriptions a write touched, and tail structured logs with the request context attached. When something's off in production, you're looking at the live system, not reconstructing it from log lines.",
			},
			{
				title: "Same tool, every environment",
				body: "Studio is part of the binary, so it runs without setup in development and behind the admin gate in production. It works the same against local, staging, and cloud deployments.",
			},
		],
		metaTitle: "Pylon Studio: browse tables, tail logs, run mutations",
		metaDescription:
			"Browse tables, inspect live queries, tail logs, and run ad-hoc mutations at /studio. Your schema supplies the UI, with no setup in development and an admin gate in production.",
		related: ["database", "functions", "cloud"],
	},

	cloud: {
		slug: "cloud",
		navLabel: "Smallware",
		category: "Smallware",
		title: "Push to GitHub. It's live.",
		tagline:
			"Smallware runs the same binary you run locally. Connect a repo and every push deploys. Resize machines, add replicas, expand regions, bring your domain, and turn on SSO from one dashboard.",
		highlights: [
			"Connect GitHub and every push to main deploys; PRs get preview environments",
			"Or run pylon deploy for air-gapped builds and CI without GitHub",
			"Resize RAM, add replicas, expand to US / EU / APAC / South America from the dashboard",
			"Custom domains + TLS, OIDC and SAML SSO, audit log, and one-click volume snapshots",
		],
		replaces: [
			"A container registry and the pipeline that fills it",
			"Load balancers, TLS certificates, and DNS wiring",
			"A metrics stack to answer what request rates and errors are doing",
			"Preview environments stitched together out of CI steps",
		],
		code: {
			label: "deploy",
			code: `$ pylon deploy --target cloud
  ✓ Build · 12s
  ✓ Schema synced
  ✓ Cutover · 0 errors
  → https://acme.smallware.run`,
		},
		sections: [
			{
				title: "The same binary, managed",
				body: "There's no special cloud build of your app. Smallware runs the identical runtime you run on localhost, so what passes in dev behaves in production. Connect a repo for push-to-deploy with preview environments per pull request, or cut releases manually with pylon deploy.",
			},
			{
				title: "Scale from the dashboard",
				body: "Bump RAM up to 64 GB, run up to 32 replicas per region, expand your volume to 500 GB, and deploy across our worldwide fleet from the dashboard without redeploying. Free projects autostop when idle; paid projects stay warm.",
			},
			{
				title: "Domains, SSO, and recovery",
				body: "Bring your own domain and Cloud handles the certificate. Configure OIDC or SAML SSO at the org level. An activity log records changes, and snapshots restore your volume. Co-located managed Postgres alongside SQLite is in private beta.",
			},
		],
		metaTitle: "Smallware: push-to-deploy managed hosting for Pylon apps",
		metaDescription:
			"Connect a repo and push. Smallware runs the same binary you run locally with scaling, global regions, custom domains, SSO, snapshots, and one dashboard.",
		related: ["ssr", "workflows", "studio"],
	},

	swift: {
		slug: "swift",
		navLabel: "Swift SDK",
		category: "Swift SDK",
		title: "Native iOS and Mac, same engine.",
		tagline:
			"The Pylon sync engine is ported to Swift and kept at parity with TypeScript. Generate a typed Swift client from your schema with live queries, optimistic writes, offline reconciliation, and a Loro CRDT bridge.",
		highlights: [
			"The full sync engine runs natively in Swift",
			"Feature parity with the TypeScript engine; sync fixes ship to both",
			"Live queries and optimistic mutations with offline reconciliation",
			"Typed client generated from your schema: pylon codegen client --target swift",
		],
		replaces: [
			"A hand-written REST client maintained per platform",
			"A local cache and the invalidation rules to go with it",
			"Offline queueing implemented separately on mobile",
			"A second data model that drifts from the web one",
		],
		code: {
			label: "Terminal",
			code: `$ pylon codegen client --target swift

# Generates a typed Swift client + models from your
# schema. Drop it into your Xcode project and call
# the same queries and mutations as on the web.`,
		},
		sections: [
			{
				title: "The sync engine in Swift",
				body: "packages/swift implements the web client's reconciliation, operation queue, optimistic rollback, and snapshot pagination in Swift. Your iOS and Mac apps get the browser's local-first behavior and guarantees through a native sync engine.",
			},
			{
				title: "Parity across clients",
				body: "The TypeScript and Swift engines are held at feature parity by policy: every sync fix lands in both. This keeps a web app and native Mac app on the same backend from drifting into platform-specific data bugs.",
			},
			{
				title: "Typed from your schema",
				body: "Run pylon codegen client --target swift and get Swift models and a typed client generated from the same schema your backend uses. A Loro CRDT bridge handles rich collaborative state. Your Swift views call the same queries and mutations by name.",
			},
		],
		metaTitle: "Pylon Swift SDK: the native sync engine for iOS and Mac",
		metaDescription:
			"The Pylon sync engine ported to Swift and kept at parity with TypeScript. Live queries, optimistic writes, offline reconciliation, a Loro bridge, and codegen from your schema.",
		related: ["sync", "realtime", "database"],
	},
};

/** Slugs in nav/index order. */
export const PRODUCT_SLUGS = Object.keys(PRODUCTS);

export function getProduct(slug: string): ProductContent | undefined {
	return PRODUCTS[slug];
}
