// Single source of truth for the marketing site's information architecture.
// The mega-menu nav, the footer, and the section-index pages (/product,
// /solutions, /developers) all read from this so the structure stays in lockstep.
//
// Mirrors the Supabase model — Product / Solutions / Developers / Compare /
// Pricing / Docs — populated with Pylon's actual offering: the framework
// primitives, Stack0 Cloud, the Swift SDK, use-case solutions, and the
// competitor comparisons. Each link carries a lucide icon so the mega-menu
// renders Supabase-style icon tiles.

import {
	type LucideIcon,
	Bot,
	BookOpen,
	Boxes,
	Braces,
	Cloud,
	Database,
	FileCode2,
	Github,
	HardDrive,
	History,
	LayoutDashboard,
	Lock,
	RefreshCw,
	Radio,
	Search,
	Server,
	Smartphone,
	Users,
	Workflow,
	Zap,
} from "lucide-react";

export interface NavLink {
	label: string;
	href: string;
	/** One-line description shown in the mega-menu. */
	desc?: string;
	/** External (full URL) — rendered as a plain <a>, not a client <Link>. */
	external?: boolean;
	/** lucide icon rendered in the mega-menu / index cards. */
	icon?: LucideIcon;
}

export interface NavGroup {
	title: string;
	links: NavLink[];
}

/** The twelve framework primitives + cloud + clients, as /product/* pages. */
export const PRODUCT_GROUPS: NavGroup[] = [
	{
		title: "Build",
		links: [
			{
				label: "Sync engine",
				href: "/product/sync",
				desc: "Live queries over WebSocket. Local-first, offline-ready.",
				icon: RefreshCw,
			},
			{
				label: "Database",
				href: "/product/database",
				desc: "Typed schema, migrations on save. SQLite or Postgres.",
				icon: Database,
			},
			{
				label: "Auth & policies",
				href: "/product/auth",
				desc: "Magic-link, 25+ OAuth, OIDC. Row-level access rules.",
				icon: Lock,
			},
			{
				label: "Server functions",
				href: "/product/functions",
				desc: "Queries, mutations, actions in TypeScript. Filename is the RPC.",
				icon: Braces,
			},
			{
				label: "Realtime",
				href: "/product/realtime",
				desc: "Rooms, presence, and authoritative tick-based shards.",
				icon: Radio,
			},
			{
				label: "Storage",
				href: "/product/storage",
				desc: "Presigned uploads to local disk or any S3 bucket.",
				icon: HardDrive,
			},
			{
				label: "Search",
				href: "/product/search",
				desc: "BM25 ranking + live facets, right in your database.",
				icon: Search,
			},
			{
				label: "Workflows & jobs",
				href: "/product/workflows",
				desc: "Durable workflows, background jobs, version-controlled cron.",
				icon: Workflow,
			},
			{
				label: "SSR",
				href: "/product/ssr",
				desc: "Native React server rendering, with <Link> and <Image>.",
				icon: Server,
			},
			{
				label: "Studio",
				href: "/product/studio",
				desc: "Browse tables, tail logs, run mutations at /studio.",
				icon: LayoutDashboard,
			},
		],
	},
	{
		title: "Ship",
		links: [
			{
				// Named for the capability, not the brand: the brand already has
				// its own entry in SOLUTIONS below, and two nav items both reading
				// "Stack0 Cloud" told the visitor nothing about the difference. This
				// one is the framework's hosting story; that one is the product.
				label: "Managed hosting",
				href: "/product/cloud",
				desc: "Push to GitHub, it's live. Scaling, regions, domains, SSO.",
				icon: Cloud,
			},
		],
	},
	{
		title: "Clients",
		links: [
			{
				label: "Swift SDK",
				href: "/product/swift",
				desc: "Native iOS & Mac — the same sync engine, in Swift.",
				icon: Smartphone,
			},
		],
	},
];

export const SOLUTIONS: NavLink[] = [
	{
		// Stack0 Cloud is its own product on its own domain now, so this leaves the
		// site rather than pointing at /smallware — that route moved with the
		// control plane and 404s here.
		label: "Stack0 Cloud",
		href: "https://cloud.stack0.dev",
		external: true,
		desc: "Purpose-built tools for a handful of users, with deployment, auth, data, and backups included.",
		icon: Boxes,
	},
	{
		label: "Local-first apps",
		href: "/solutions/local-first",
		desc: "Instant, offline-capable apps that sync when they reconnect.",
		icon: Zap,
	},
	{
		label: "Realtime collaboration",
		href: "/solutions/collaboration",
		desc: "Multiplayer docs, boards, and presence out of the box.",
		icon: Users,
	},
	{
		label: "AI apps & agents",
		href: "/solutions/ai-apps",
		desc: "ctx.llm + live state for agentic, streaming experiences.",
		icon: Bot,
	},
	{
		label: "Mobile (Swift)",
		href: "/solutions/mobile",
		desc: "Native iOS/Mac with the first-class Swift SDK + Loro bridge.",
		icon: Smartphone,
	},
];

export const DEVELOPERS: NavLink[] = [
	{
		label: "Documentation",
		href: "https://docs.pylonsync.com",
		desc: "Guides, the schema model, the policy DSL, the API reference.",
		external: true,
		icon: BookOpen,
	},
	{
		label: "Examples",
		href: "/developers/examples",
		desc: "Production-shaped apps: chat, marketplace, multiplayer, more.",
		icon: Boxes,
	},
	{
		label: "Claude skill",
		href: "/skill",
		desc: "One file that teaches Claude Code to write Pylon that compiles.",
		icon: FileCode2,
	},
	{
		label: "Developer resources",
		href: "/developers",
		desc: "Docs, the CLI, templates, the MCP server, and the agent API spec.",
		icon: Braces,
	},
	{
		label: "Changelog",
		href: "https://github.com/pylonsync/pylon/releases",
		desc: "What shipped, release by release.",
		external: true,
		icon: History,
	},
	{
		label: "GitHub",
		href: "https://github.com/pylonsync/pylon",
		desc: "Star the repo, read the source, open an issue.",
		external: true,
		icon: Github,
	},
];

/** Competitor comparisons (/vs/*). slug → display name. */
export const COMPARISONS: NavLink[] = [
	{ label: "Pylon vs Supabase", href: "/vs/supabase" },
	{ label: "Pylon vs Convex", href: "/vs/convex" },
	{ label: "Pylon vs Firebase", href: "/vs/firebase" },
	{ label: "Pylon vs InstantDB", href: "/vs/instantdb" },
	{ label: "All comparisons", href: "/vs" },
];

/** Top-level nav items, in order. */
export const TOP_NAV = [
	{ label: "Product", key: "product" },
	{ label: "Solutions", key: "solutions" },
	{ label: "Developers", key: "developers" },
	{ label: "Compare", key: "compare" },
	{ label: "Docs", href: "https://docs.pylonsync.com", external: true },
] as const;
