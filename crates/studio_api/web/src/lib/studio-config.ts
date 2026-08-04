// Studio config types + defaults — mirror of packages/sdk/src/studio.ts.
//
// We can't import @pylon/sdk here directly because the studio web bundle is
// shipped self-contained with the framework binary; reaching into a sibling
// workspace package would only confuse the build. Keeping the shapes in
// sync is enforced by the kernel-side struct (which serializes to the same
// JSON wire format) and by the fact that both files are edited together.

export type ThemeAccent = "emerald" | "blue" | "violet" | "rose" | "amber";
export type ThemeAppearance = "dark" | "light" | "system";

export interface BrandConfig {
	name?: string;
	logo?: string;
	subtitle?: string;
}

export interface ThemeConfig {
	accent?: ThemeAccent;
	appearance?: ThemeAppearance;
	primary?: string;
}

export type SidebarItem =
	| { type: "page"; id: string; label: string; icon?: string; requiresAdmin?: boolean; requiresRoles?: string[] }
	| { type: "resource"; entity: string; label?: string; icon?: string; requiresAdmin?: boolean; requiresRoles?: string[] }
	| { type: "link"; label: string; href: string; icon?: string; external?: boolean }
	| { type: "heading"; label: string };

export interface SidebarSection {
	label: string;
	items: SidebarItem[];
	defaultOpen?: boolean;
}

export type SidebarFooter =
	| {
			type: "card";
			title: string;
			description: string;
			action?: { label: string; href: string };
			progress?: number;
	  }
	| { type: "custom"; componentId: string };

export interface SidebarConfig {
	sections?: SidebarSection[];
	footer?: SidebarFooter | null;
	orgSwitcher?: { items: { id: string; label: string }[] } | null;
	collapsible?: boolean;
}

export type BadgeVariant = "green" | "red" | "amber" | "blue" | "gray";

export type ColumnRenderer =
	| { kind: "text"; truncate?: number; mono?: boolean }
	| { kind: "avatar"; imageField?: string; subtitleField?: string; nameField?: string }
	| { kind: "badge"; variants?: Record<string, BadgeVariant>; dot?: boolean }
	| { kind: "date"; format?: "relative" | "absolute" | "iso" }
	| { kind: "link"; href: string; external?: boolean }
	| { kind: "boolean"; trueLabel?: string; falseLabel?: string }
	| {
			kind: "number";
			style?: "decimal" | "percent" | "currency" | "bytes";
			currency?: string;
			locale?: string;
	  }
	| { kind: "json"; truncate?: number }
	| { kind: "custom"; componentId: string };

export interface ColumnConfig {
	field: string;
	label?: string;
	order?: number;
	hidden?: boolean;
	sortable?: boolean;
	searchable?: boolean;
	filterable?: boolean | { options?: { label: string; value: unknown }[] };
	renderer?: ColumnRenderer;
	width?: string;
	align?: "left" | "center" | "right";
}

export interface BulkAction {
	id: string;
	label: string;
	icon?: string;
	kind?: "delete" | "export" | "custom";
	confirm?: string;
	requiresAdmin?: boolean;
}

export interface RowAction {
	id: string;
	label: string;
	icon?: string;
	kind?: "delete" | "edit" | "view" | "action" | "custom";
	display?: "menu" | "button";
	/** Function name for `kind: "action"`. */
	action?: string;
	/** Args, with `{row.<field>}` placeholders. Defaults to `{ id }`. */
	input?: Record<string, unknown>;
	result?: "toast" | "copy" | "dialog" | "none";
	resultField?: string;
	refresh?: boolean;
	variant?: "default" | "outline" | "ghost" | "secondary" | "destructive";
	confirm?: string;
	requiresAdmin?: boolean;
}

export interface ResourceListConfig {
	columns?: ColumnConfig[];
	searchable?: boolean;
	filterable?: boolean;
	bulkActions?: BulkAction[];
	rowActions?: RowAction[];
	defaultSort?: { field: string; order: "asc" | "desc" };
	pageSizes?: number[];
	defaultPageSize?: number;
}

export interface ResourceConfig {
	label?: string;
	pluralLabel?: string;
	icon?: string;
	hidden?: boolean;
	list?: ResourceListConfig;
}

export interface PageConfig {
	subtitle?: string;
	componentId?: string;
}

export interface StudioConfig {
	brand?: BrandConfig;
	theme?: ThemeConfig;
	sidebar?: SidebarConfig;
	resources?: Record<string, ResourceConfig>;
	pages?: Record<string, PageConfig>;
	hasExtensions?: boolean;
}

declare global {
	interface Window {
		__PYLON_STUDIO_CONFIG__?: StudioConfig;
	}
}

/**
 * Read the injected config and return a fully-populated, defensible
 * shape. Anything missing is resolved against built-in defaults — the
 * UI never has to handle `undefined` config branches.
 */
export function resolveConfig(): StudioConfig {
	if (typeof window === "undefined") return {};
	return window.__PYLON_STUDIO_CONFIG__ ?? {};
}
