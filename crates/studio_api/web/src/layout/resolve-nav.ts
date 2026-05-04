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

/// Built-in page ids the layout recognises. Custom ids resolve through
/// the extensions registry — see `extensions.ts`.
export const BUILT_IN_PAGES = new Set([
	"overview",
	"manifest",
	"functions",
	"policies",
	"routes",
	"sync",
	"health",
	"settings",
	"roles",
]);

/**
 * Build the sidebar nav from the user's config when present, or
 * fall back to a manifest-derived default that mirrors the
 * Refine-style screenshot:
 *
 *   DASHBOARD
 *     • Overview
 *   RESOURCES
 *     • <every entity in manifest, except hidden>
 *   SETTINGS
 *     • Roles & Permissions
 *
 * The default flow keeps existing pages reachable (Health / Sync /
 * Manifest / Functions / Policies / Routes / Settings) under a
 * "FRAMEWORK" section so the operator never loses access to admin
 * surfaces just because they didn't write a studio.config.ts yet.
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
	const sections: ResolvedNavSection[] = [];

	sections.push({
		label: "DASHBOARD",
		defaultOpen: true,
		items: [
			{ kind: "page", id: "overview", label: "Overview", icon: "grid" },
		],
	});

	const visibleEntities = manifest.entities.filter(
		(e) => !config.resources?.[e.name]?.hidden,
	);
	if (visibleEntities.length > 0) {
		sections.push({
			label: "RESOURCES",
			defaultOpen: true,
			items: visibleEntities.map((e) => {
				const meta = config.resources?.[e.name];
				return {
					kind: "resource",
					entity: e.name,
					label: meta?.label ?? meta?.pluralLabel ?? e.name,
					icon: meta?.icon ?? "layers",
				};
			}),
		});
	}

	sections.push({
		label: "FRAMEWORK",
		defaultOpen: true,
		items: [
			{
				kind: "page",
				id: "manifest",
				label: "Manifest",
				icon: "file-text",
			},
			{
				kind: "page",
				id: "functions",
				label: "Functions",
				icon: "file-code",
				requiresAdmin: true,
			},
			{ kind: "page", id: "policies", label: "Policies", icon: "shield-check" },
			{ kind: "page", id: "routes", label: "Routes", icon: "box" },
			{ kind: "page", id: "sync", label: "Live sync", icon: "radio" },
			{
				kind: "page",
				id: "health",
				label: "Health",
				icon: "activity",
				requiresAdmin: true,
			},
		],
	});

	sections.push({
		label: "SETTINGS",
		defaultOpen: true,
		items: [
			{ kind: "page", id: "settings", label: "Settings", icon: "settings" },
		],
	});

	return sections;
}

/// Default footer card (matches the screenshot's "Used space" pattern)
/// when the user hasn't configured one. The progress value is a
/// placeholder — the real numeric ratio comes from server metrics
/// when available, otherwise we render the description text only.
export function defaultFooter(config: SidebarConfig | undefined) {
	if (config?.footer === null) return null; // explicit opt-out
	return (
		config?.footer ?? {
			type: "card" as const,
			title: "Pylon Studio",
			description:
				"Configure brand, resources, and theme via studio.config.ts",
			action: { label: "Read the docs", href: "https://docs.pylonsync.com" },
		}
	);
}
