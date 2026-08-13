// Example apps, extracted from the examples page so the /product/* pages can
// show the ones that exercise their primitive. The templates are real
// create-pylon templates; the live demos are deployed Pylon apps. We don't
// link demos we can't stand behind. Keep in sync with the TEMPLATE_REGISTRY in
// pylon's packages/create-pylon/bin/create-pylon.js.

export interface Example {
	name: string;
	blurb: string;
	shows: string[];
	/**
	 * A real template name (run npm create, pick it) — or undefined.
	 *
	 * This must be the template's DIRECTORY name, not one of the retired
	 * aliases in TEMPLATE_ALIASES. `templateRepoUrl` builds a repo path from
	 * it, and an alias resolves to a directory that doesn't exist.
	 */
	template?: string;
	/** A live demo URL — or undefined. */
	live?: string;
	/**
	 * Screenshot of the template running, served from the site's public dir.
	 * Undefined means no capture exists yet and the card draws a placeholder
	 * instead. Populate with `scripts/capture-template-shots.mjs`.
	 */
	shot?: string;
}

/** Where a template's source lives on GitHub. */
export function templateRepoUrl(template: string): string {
	return `https://github.com/pylonsync/pylon/tree/main/packages/create-pylon/templates/${template}`;
}

/**
 * The copy/paste command that scaffolds a template.
 *
 * Flag form matches what `create-pylon --help` documents; `@latest` matches
 * the hero's install pill so a stale cached scaffolder can't produce a
 * template the docs no longer describe.
 */
export function createCommand(template: string): string {
	return `npm create @pylonsync/pylon@latest my-app --template ${template}`;
}

/**
 * The templates the landing page shows, in order. A subset chosen for
 * breadth — a product starter, realtime, commerce, search, AI, a two-sided
 * marketplace, and two business sites — not a ranking. The full set lives on
 * /developers/examples.
 */
export const FEATURED_TEMPLATES = [
	"saas",
	"chat",
	"shop",
	"directory",
	"ai-chat",
	"marketplace",
	"agency",
	"waitlist",
] as const;

/**
 * The featured examples, resolved from the same GROUPS + LIVE_DEMOS data the
 * examples page renders. Deriving them keeps one description per template;
 * a second hand-maintained list is how the two pages would drift.
 *
 * A name in FEATURED_TEMPLATES with no matching example is skipped rather
 * than rendered empty.
 */
export function featuredExamples(): Example[] {
	const all = [...LIVE_DEMOS, ...GROUPS.flatMap((g) => g.items)];
	return FEATURED_TEMPLATES.map((t) =>
		all.find((e) => e.template === t),
	).filter((e): e is Example => Boolean(e));
}

export interface Group {
	label: string;
	blurb: string;
	items: Example[];
}

// Live, deployed demos — real Pylon apps running on Smallware. Verified up
// before shipping; we don't link demos we can't stand behind.
export const LIVE_DEMOS: Example[] = [
	{
		name: "Marketplace",
		blurb:
			"A trading marketplace deployed from a monorepo, with listings, watchlists, an activity feed, faceted search, and per-user ownership.",
		shows: ["Faceted search", "field.owner()", "Optimistic inserts"],
		live: "https://market.pyln.dev",
		template: "marketplace",
	},
	{
		name: "World3D",
		blurb:
			"A multiplayer procedural island with three.js in the browser and Pylon SSR and realtime sync on the server. Every visitor moves through the same world.",
		shows: ["Realtime multiplayer", "three.js + SSR", "Live sync"],
		live: "https://world3d.pyln.dev",
	},
	{
		name: "Co-op city builder",
		blurb:
			"A shared SimCity-style builder on three.js + Pylon SSR. Every open tab is a co-mayor building the same city live.",
		shows: ["Shared state", "three.js + SSR", "Realtime"],
		live: "https://sim.pyln.dev",
	},
	{
		name: "Pad",
		blurb:
			"A collaborative markdown editor in one entity, two pages, and about 400 lines. Open a doc in two windows and watch edits merge through a text CRDT.",
		shows: ["Text CRDT", "Live collaboration", "~400 lines"],
		live: "https://pad.smallware.run",
	},
];

export const GROUPS: Group[] = [
	{
		label: "Full-stack starters",
		blurb: "Pick a starter to see Pylon's core primitives working end to end.",
		items: [
			{
				name: "SaaS starter",
				blurb:
					"A complete SaaS product with a marketing site, onboarding, a multi-tenant dashboard, and Stripe billing. Reach for this one to build a product.",
				shows: ["Stripe billing", "Marketing + dashboard", "Onboarding"],
				// "default" is a retired alias kept working by TEMPLATE_ALIASES.
				// The directory is `saas`, and templateRepoUrl() builds a path from
				// this value — an alias here points at a directory that isn't there.
				template: "saas",
			},
			{
				name: "Consumer app",
				blurb:
					"A social feed with live posts and likes, public reads, and owner-only writes.",
				shows: ["Live feed", "Public reads", "field.owner()"],
				template: "consumer",
			},
			{
				name: "Realtime chat",
				blurb:
					"Rooms and messages with live queries and presence. The canonical 'see sync work' starter.",
				shows: ["Live queries", "Presence", "Optimistic send"],
				template: "chat",
			},
			{
				name: "Local-first todo",
				blurb:
					"Optimistic create / toggle / delete with guest sessions. The smallest end-to-end Pylon app.",
				shows: ["Optimistic writes", "Guest sessions", "Offline-ready"],
				template: "todo",
			},
			{
				name: "Barebones",
				blurb:
					"One entity, one function, one page. Start from a clean slate.",
				shows: ["Schema", "One function", "SSR page"],
				template: "barebones",
			},
		],
	},
	{
		label: "Business websites",
		blurb:
			"A configurable marketing site, real-time feature, and owner dashboard, ready to rebrand.",
		items: [
			{
				name: "Waitlist",
				blurb:
					"A coming-soon landing with email capture and a live signup counter that ticks up across every open tab.",
				shows: ["Live counter", "Email capture", "Owner dashboard"],
				template: "waitlist",
			},
			{
				name: "Agency / studio",
				blurb:
					"A studio site with a case-study portfolio, plus a back-office: clients, invoices with PDF export, and live project availability.",
				shows: ["Case studies", "Invoices + PDF", "Live availability"],
				template: "agency",
			},
			{
				name: "Restaurant",
				blurb:
					"Menu and reservations with live table availability. Each seating greys to 'Full' when it fills.",
				shows: ["Live availability", "Reservations", "Owner dashboard"],
				template: "restaurant",
			},
			{
				name: "Local service",
				blurb:
					"An appointment business with services, booking, live slot availability, and an owner dashboard.",
				shows: ["Live booking", "Scheduling", "Owner dashboard"],
				template: "local-service",
			},
			{
				name: "Creator",
				blurb:
					"A personal-brand page with a bio, offerings, newsletter signup, and live subscriber count.",
				shows: ["Live counter", "Newsletter", "Owner dashboard"],
				template: "creator",
			},
			{
				name: "Shop",
				blurb:
					"A DTC store with a product grid, live inventory, a cart, and real Stripe checkout with a no-key fallback.",
				shows: ["Stripe checkout", "Live inventory", "Cart"],
				template: "shop",
			},
			{
				name: "Directory",
				blurb:
					"A curated directory with live full-text search, facets, community upvotes, and a moderated submission flow.",
				shows: ["Full-text search", "Facets", "Moderation"],
				template: "directory",
			},
		],
	},
	{
		label: "AI apps",
		blurb: "Pylon keeps the AI calls on the server, so your keys never reach the browser.",
		items: [
			{
				name: "AI chat",
				blurb:
					"Streaming LLM chat keeps your key on the server and syncs conversation history across tabs. Pick from multiple models.",
				shows: ["Token streaming", "Synced history", "Multi-model"],
				template: "ai-chat",
			},
			{
				name: "AI studio",
				blurb:
					"Generate images, audio, and video with Replicate. A live gallery fills in as each background job finishes.",
				shows: ["Background jobs", "Live gallery", "Replicate"],
				template: "ai-studio",
			},
		],
	},
];

// Which primitive each example exercises, derived from the `shows` tags the
// examples already carry rather than a hand-maintained second list — so an
// example can never claim a primitive its own tags don't mention.
const TAG_TO_SLUG: Record<string, string> = {
	"Faceted search": "search",
	"Full-text search": "search",
	Facets: "search",
	"field.owner()": "auth",
	"Public reads": "auth",
	"Guest sessions": "auth",
	Onboarding: "auth",
	"Optimistic inserts": "sync",
	"Optimistic writes": "sync",
	"Optimistic send": "sync",
	"Live sync": "sync",
	"Live queries": "sync",
	"Live feed": "sync",
	"Live counter": "sync",
	"Live inventory": "sync",
	"Live availability": "sync",
	"Live gallery": "sync",
	"Offline-ready": "sync",
	"Realtime multiplayer": "realtime",
	Realtime: "realtime",
	Presence: "realtime",
	"Shared state": "realtime",
	"Live collaboration": "realtime",
	"Text CRDT": "realtime",
	"three.js + SSR": "ssr",
	"SSR page": "ssr",
	"Marketing + dashboard": "ssr",
	"Background jobs": "workflows",
	Scheduling: "workflows",
	"One function": "functions",
	"Token streaming": "functions",
	"Stripe billing": "functions",
	"Stripe checkout": "functions",
	Schema: "database",
	"Invoices + PDF": "storage",
};

/** Examples whose own tags name this primitive. Live demos first. */
export function examplesFor(slug: string): Example[] {
	const all = [...LIVE_DEMOS, ...GROUPS.flatMap((g) => g.items)];
	const matches = all.filter((e) =>
		e.shows.some((tag) => TAG_TO_SLUG[tag] === slug),
	);
	return [
		...matches.filter((e) => e.live),
		...matches.filter((e) => !e.live),
	].slice(0, 3);
}
