import type {
	SidebarConfig,
	SidebarItem,
	SidebarSection,
	StudioConfig,
} from "@/lib/studio-config";
import type { Manifest } from "@/lib/pylon";

export type ResolvedNavItem =
	| { kind: "page"; id: string; label: string; icon?: string; requiresAdmin?: boolean }
	| {
			kind: "resource";
			entity: string;
			label: string;
			icon?: string;
			requiresAdmin?: boolean;
	  }
	| { kind: "link"; label: string; href: string; icon?: string; external: boolean }
	| { kind: "heading"; label: string };

export interface ResolvedNavSection {
	label: string;
	items: ResolvedNavItem[];
	defaultOpen: boolean;
}

/// The ids of the built-in pages. Any other id is looked up in the
/// extensions registry (`extensions.ts`).
export const BUILT_IN_PAGES = new Set([
	"overview",
	"manifest",
	"functions",
	"policies",
	"routes",
	"sync",
	"health",
	"shards",
	"settings",
	"roles",
	"entities",
	"logs",
	"metrics",
	"jobs",
	"workflows",
]);

/**
 * Build the sidebar groups.
 *
 * `sidebar.sections` from studio.config.ts wins when it has entries. With no
 * config the groups come from the manifest:
 *
 *   (none)   Overview, the home page of Studio
 *   Data     one item for each entity that is not hidden
 *   App      Functions, Policies, Routes, Manifest, Roles
 *   Runtime  Logs, Metrics, Jobs, Workflows, Live sync, Health, Shards,
 *            System tables
 */
export function resolveNav(
	config: StudioConfig,
	manifest: Manifest,
): ResolvedNavSection[] {
	if (config.sidebar?.sections && config.sidebar.sections.length > 0) {
		return config.sidebar.sections.map((s) => normalizeSection(s, config));
	}
	return defaultSidebar(manifest, config);
}

function normalizeSection(
	section: SidebarSection,
	config: StudioConfig,
): ResolvedNavSection {
	return {
		label: section.label,
		defaultOpen: section.defaultOpen ?? true,
		items: section.items.map((item) => normalizeItem(item, config)),
	};
}

function normalizeItem(item: SidebarItem, config: StudioConfig): ResolvedNavItem {
	switch (item.type) {
		case "page":
			return {
				kind: "page",
				id: item.id,
				label: item.label,
				icon: item.icon,
				requiresAdmin: item.requiresAdmin,
			};
		case "resource": {
			const meta = config.resources?.[item.entity];
			return {
				kind: "resource",
				entity: item.entity,
				label: item.label ?? meta?.label ?? item.entity,
				icon: item.icon ?? meta?.icon,
				requiresAdmin: item.requiresAdmin,
			};
		}
		case "link":
			return {
				kind: "link",
				label: item.label,
				href: item.href,
				icon: item.icon,
				external:
					item.external ??
					(/^https?:\/\//i.test(item.href) ||
						item.href.startsWith("//")),
			};
		case "heading":
			return { kind: "heading", label: item.label };
	}
}

function defaultSidebar(
	manifest: Manifest,
	config: StudioConfig,
): ResolvedNavSection[] {
	const sections: ResolvedNavSection[] = [
		{
			label: "",
			defaultOpen: true,
			items: [{ kind: "page", id: "overview", label: "Overview", icon: "bar-chart" }],
		},
	];

	const visibleEntities = manifest.entities.filter(
		(e) => !config.resources?.[e.name]?.hidden,
	);
	if (visibleEntities.length > 0) {
		sections.push({
			label: "Data",
			defaultOpen: true,
			items: visibleEntities.map((e) => {
				const meta = config.resources?.[e.name];
				return {
					kind: "resource",
					entity: e.name,
					label: meta?.label ?? meta?.pluralLabel ?? e.name,
					icon: meta?.icon ?? "table",
				};
			}),
		});
	}

	sections.push({
		label: "App",
		defaultOpen: true,
		items: [
			{
				kind: "page",
				id: "functions",
				label: "Functions",
				icon: "file-code",
				requiresAdmin: true,
			},
			{ kind: "page", id: "policies", label: "Policies", icon: "shield-check" },
			{ kind: "page", id: "routes", label: "Routes", icon: "route" },
			{ kind: "page", id: "manifest", label: "Manifest", icon: "file-text" },
			{ kind: "page", id: "roles", label: "Roles", icon: "user-cog" },
		],
	});

	sections.push({
		label: "Runtime",
		defaultOpen: true,
		items: [
			{ kind: "page", id: "logs", label: "Logs", icon: "scroll-text", requiresAdmin: true },
			{ kind: "page", id: "metrics", label: "Metrics", icon: "gauge", requiresAdmin: true },
			{ kind: "page", id: "jobs", label: "Jobs", icon: "list-checks", requiresAdmin: true },
			{
				kind: "page",
				id: "workflows",
				label: "Workflows",
				icon: "workflow",
				requiresAdmin: true,
			},
			{ kind: "page", id: "sync", label: "Live sync", icon: "radio", requiresAdmin: true },
			{
				kind: "page",
				id: "health",
				label: "Health",
				icon: "activity",
				requiresAdmin: true,
			},
			{
				kind: "page",
				id: "shards",
				label: "Shards",
				icon: "boxes",
				requiresAdmin: true,
			},
			{
				kind: "page",
				id: "entities",
				label: "System tables",
				icon: "database",
				requiresAdmin: true,
			},
		],
	});

	return sections;
}

/** The footer from studio.config.ts. With no config the sidebar has no footer. */
export function resolveFooter(config: SidebarConfig | undefined) {
	return config?.footer ?? null;
}
