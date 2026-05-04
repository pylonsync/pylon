// ---------------------------------------------------------------------------
// Pylon Studio configuration
// ---------------------------------------------------------------------------
//
// Authored as `studio.config.ts` in the user's pylon project. The CLI runs
// `bun run studio.config.ts` which prints the result of `defineStudioConfig`
// to stdout as JSON. The runtime reads `.pylon/studio.config.json` at boot
// and injects it into the Studio HTML as `window.__PYLON_STUDIO_CONFIG__`.
//
// Keep every field here JSON-serializable. React components (custom column
// renderers, custom pages, custom layout slots) live in the *separate*
// `studio.entry.tsx` file and are referenced from the config by string id.
// `studio.entry.tsx` is bundled to ESM and dynamically imported by the
// Studio shell at boot — see `defineStudioExtensions` below.

// ---------------------------------------------------------------------------
// Brand
// ---------------------------------------------------------------------------

export interface BrandConfig {
  /** Display name in the sidebar header. Defaults to manifest.name. */
  name?: string;
  /**
   * Logo source. One of:
   *   - data URL (`data:image/svg+xml;base64,...`)
   *   - absolute URL (`https://...`)
   *   - path served by your app (`/assets/logo.svg`)
   *   - emoji or single grapheme rendered as a glyph (`"⚡"`)
   * Omit to fall back to the bolt icon.
   */
  logo?: string;
  /** Optional subtitle shown under the brand name (e.g. "v1.2.3"). */
  subtitle?: string;
}

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

/** Built-in accent palettes. The Studio ships CSS variables for each. */
export type ThemeAccent = "emerald" | "blue" | "violet" | "rose" | "amber";

/** Initial appearance — the user can still toggle via the UI (next-themes). */
export type ThemeAppearance = "dark" | "light" | "system";

export interface ThemeConfig {
  accent?: ThemeAccent;
  appearance?: ThemeAppearance;
  /**
   * Override the accent's primary color with a custom hex. When set, the
   * accent CSS variables are derived from this value instead of the
   * built-in palette. Use sparingly — the built-in palettes are tuned for
   * AA contrast in both dark and light.
   */
  primary?: string;
}

// ---------------------------------------------------------------------------
// Sidebar
// ---------------------------------------------------------------------------

/** Lucide icon name (kebab-case). Validated client-side. */
export type IconName = string;

export interface SidebarPageItem {
  type: "page";
  /**
   * Built-in page id (`overview`, `users`, `roles`, `health`, `sync`,
   * `manifest`, `functions`, `policies`, `routes`, `settings`) OR a key
   * registered by `studio.entry.tsx` via `defineStudioExtensions({ pages })`.
   */
  id: string;
  label: string;
  icon?: IconName;
  /** Hide unless the current viewer is_admin. */
  requiresAdmin?: boolean;
  /** Hide unless the current viewer has at least one of these roles. */
  requiresRoles?: string[];
}

export interface SidebarResourceItem {
  type: "resource";
  /** Manifest entity name. The list page is auto-generated. */
  entity: string;
  /** Override the sidebar label. Defaults to entity name. */
  label?: string;
  icon?: IconName;
  requiresAdmin?: boolean;
  requiresRoles?: string[];
}

export interface SidebarLinkItem {
  type: "link";
  label: string;
  href: string;
  icon?: IconName;
  /** Open in a new tab. Defaults to true for absolute URLs. */
  external?: boolean;
}

export interface SidebarHeadingItem {
  type: "heading";
  label: string;
}

export type SidebarItem =
  | SidebarPageItem
  | SidebarResourceItem
  | SidebarLinkItem
  | SidebarHeadingItem;

export interface SidebarSection {
  /** Uppercase label rendered above the items. Pass empty string to omit. */
  label: string;
  items: SidebarItem[];
  /** Default open state for the collapse toggle. Defaults to true. */
  defaultOpen?: boolean;
}

// ---------------------------------------------------------------------------
// Sidebar footer
// ---------------------------------------------------------------------------

export interface SidebarFooterCard {
  type: "card";
  title: string;
  description: string;
  /** Optional CTA at the bottom of the card. */
  action?: { label: string; href: string };
  /** Optional progress bar (0..1). Useful for "used space" cards. */
  progress?: number;
}

export interface SidebarFooterCustom {
  type: "custom";
  /** Component id registered via `defineStudioExtensions({ layouts: { footer: { id: ... }}})`. */
  componentId: string;
}

export type SidebarFooter = SidebarFooterCard | SidebarFooterCustom | null;

export interface SidebarConfig {
  sections: SidebarSection[];
  footer?: SidebarFooter;
  /** Org switcher button at the top. Pass `null` to hide. */
  orgSwitcher?: { items: { id: string; label: string }[] } | null;
  /** Show the collapse-to-icons toggle. Defaults to true. */
  collapsible?: boolean;
}

// ---------------------------------------------------------------------------
// Resource list configuration
// ---------------------------------------------------------------------------

/** Built-in cell renderer kinds. Custom renderers register an id under `componentId`. */
export type RendererKind =
  | "text"
  | "avatar"
  | "badge"
  | "date"
  | "link"
  | "boolean"
  | "number"
  | "json"
  | "custom";

export interface RendererText {
  kind: "text";
  /** Truncate long strings to this many chars. Default 80. */
  truncate?: number;
  /** Render as monospace. */
  mono?: boolean;
}

export interface RendererAvatar {
  kind: "avatar";
  /** Field on the row holding the image URL. If omitted, initials only. */
  imageField?: string;
  /** Field holding the secondary line under the name. */
  subtitleField?: string;
  /** Source field for the displayed name. Defaults to the column field. */
  nameField?: string;
}

export interface RendererBadge {
  kind: "badge";
  /**
   * Map of value → variant. Variants: green, red, amber, blue, gray.
   * Unknown values render gray. Use `*` as a fallback key.
   */
  variants?: Record<string, "green" | "red" | "amber" | "blue" | "gray">;
  /** Render a leading dot before the label. Defaults to true. */
  dot?: boolean;
}

export interface RendererDate {
  kind: "date";
  /** "relative" → "3 days ago"; "absolute" → "May 3, 2026"; "iso" → raw. */
  format?: "relative" | "absolute" | "iso";
}

export interface RendererLink {
  kind: "link";
  /**
   * Template for the href. `{value}` and `{row.field}` are interpolated.
   * Example: `"mailto:{value}"` or `"/users/{row.id}"`.
   */
  href: string;
  external?: boolean;
}

export interface RendererBoolean {
  kind: "boolean";
  trueLabel?: string;
  falseLabel?: string;
}

export interface RendererNumber {
  kind: "number";
  /** "decimal" | "percent" | "currency" | "bytes". Default "decimal". */
  style?: "decimal" | "percent" | "currency" | "bytes";
  /** ISO currency code when style="currency". */
  currency?: string;
  /** Locale for Intl.NumberFormat. Default browser locale. */
  locale?: string;
}

export interface RendererJson {
  kind: "json";
  /** Truncate JSON preview length. Default 60. */
  truncate?: number;
}

export interface RendererCustom {
  kind: "custom";
  /** Id registered via `defineStudioExtensions({ renderers: { ... } })`. */
  componentId: string;
}

export type ColumnRenderer =
  | RendererText
  | RendererAvatar
  | RendererBadge
  | RendererDate
  | RendererLink
  | RendererBoolean
  | RendererNumber
  | RendererJson
  | RendererCustom;

export interface ColumnConfig {
  /** Field name on the row. Supports dotted paths (`profile.name`). */
  field: string;
  /** Header label. Defaults to humanized field name. */
  label?: string;
  /** Display order in the table. Lower = earlier. Defaults to definition order. */
  order?: number;
  /** Hide column by default; user can toggle in the column picker. */
  hidden?: boolean;
  /** Server-side sort supported. */
  sortable?: boolean;
  /** Include in the search input's matched fields. */
  searchable?: boolean;
  /** Show a filter chip in the toolbar. */
  filterable?: boolean | { options?: { label: string; value: unknown }[] };
  /** Cell renderer config. Defaults to text. */
  renderer?: ColumnRenderer;
  /** Width hint (CSS, e.g. "200px" or "30%"). */
  width?: string;
  /** Right-align the column (good for numbers). */
  align?: "left" | "center" | "right";
}

export interface BulkAction {
  id: string;
  label: string;
  icon?: IconName;
  /** Built-in action ids: "delete" | "export". Otherwise resolved against extensions. */
  kind?: "delete" | "export" | "custom";
  /** Confirmation dialog message. */
  confirm?: string;
  /** Hide unless viewer is_admin. */
  requiresAdmin?: boolean;
}

export interface RowAction {
  id: string;
  label: string;
  icon?: IconName;
  kind?: "delete" | "edit" | "view" | "custom";
  confirm?: string;
  requiresAdmin?: boolean;
}

export interface ResourceListConfig {
  /** Columns. If omitted, columns are inferred from the manifest entity fields. */
  columns?: ColumnConfig[];
  /** Search box in the toolbar. Defaults to true if any column is searchable. */
  searchable?: boolean;
  /** Filters dropdown in the toolbar. Defaults to true if any column is filterable. */
  filterable?: boolean;
  /** Bulk action menu (shown when rows are selected). */
  bulkActions?: BulkAction[];
  /** Per-row trailing action menu. */
  rowActions?: RowAction[];
  /** Default sort: field + direction. */
  defaultSort?: { field: string; order: "asc" | "desc" };
  /** Page size options. Default [10, 25, 50, 100]. */
  pageSizes?: number[];
  /** Initial page size. Default 10. */
  defaultPageSize?: number;
}

export interface ResourceConfig {
  /** Sidebar / breadcrumb label override. Defaults to entity name. */
  label?: string;
  /** Plural label for headings ("Users" vs "User"). Defaults to label + "s". */
  pluralLabel?: string;
  icon?: IconName;
  /** Hide from the auto-derived sidebar. The user's explicit sidebar config still wins. */
  hidden?: boolean;
  list?: ResourceListConfig;
}

// ---------------------------------------------------------------------------
// Pages — built-in + custom
// ---------------------------------------------------------------------------

export interface PageConfig {
  /** Optional subtitle under the page header. */
  subtitle?: string;
  /** Custom component id (extensions). Required for non-built-in ids. */
  componentId?: string;
}

// ---------------------------------------------------------------------------
// Top-level config
// ---------------------------------------------------------------------------

export interface StudioConfig {
  brand?: BrandConfig;
  theme?: ThemeConfig;
  sidebar?: SidebarConfig;
  resources?: Record<string, ResourceConfig>;
  pages?: Record<string, PageConfig>;
  /**
   * When true, the Studio will dynamic-import `/studio/extensions.js` at
   * boot. The CLI sets this automatically when `studio.entry.tsx` is
   * present in the project, so users rarely set it explicitly.
   */
  hasExtensions?: boolean;
  /**
   * URL to send unauthenticated callers to when they hit `/studio`.
   * Lets a host app (Pylon Cloud, an enterprise dashboard) point the
   * Studio gate at its own email/password login page instead of the
   * built-in `/studio/login` admin-token form.
   *
   * The framework appends `?next=/studio` so the host app can redirect
   * back after sign-in. Authenticated-but-not-admin users still see
   * the framework's "access denied" page (no point sending them back
   * to a login they're already past).
   *
   * Example: `loginUrl: "/login"` — cloud.pylonsync.com handles `/login`
   * at the dashboard, and the user's existing session cookie lifts
   * them to admin via `auth.user.adminField` on the way back.
   */
  loginUrl?: string;
}

/**
 * Identity helper for `studio.config.ts`. Provides type checking + lets
 * the SDK evolve the shape under one declaration. The returned object is
 * what the CLI serializes to JSON.
 *
 * ```ts
 * // studio.config.ts
 * import { defineStudioConfig } from "@pylon/sdk";
 * export default defineStudioConfig({
 *   brand: { name: "Acme" },
 *   theme: { accent: "emerald", appearance: "dark" },
 *   sidebar: { sections: [...] },
 *   resources: { User: { list: { columns: [...] } } },
 * });
 * ```
 *
 * The CLI's bun runner expects the file's *default export* to be the
 * config object, OR the module to print it via `console.log`. The
 * recommended pattern is `console.log(JSON.stringify(defineStudioConfig({...})))`
 * — the `pylon-studio-config.mjs` shim added by `pylon dev` does this
 * automatically when you `export default`.
 */
export function defineStudioConfig(config: StudioConfig): StudioConfig {
  return config;
}

// ---------------------------------------------------------------------------
// Extensions (studio.entry.tsx)
// ---------------------------------------------------------------------------
//
// Custom renderers, custom pages, custom layout slots — anything that's a
// React component lives here. The file is bundled to ESM and dynamic-
// imported at Studio boot. It must call `registerStudioExtensions` (the
// global hook) OR default-export the registry — the bundle entry shim
// supports both.

/**
 * Component shape for column renderers. The Studio passes the cell value
 * + the full row + the column config; the renderer returns a React node.
 *
 * Kept as `unknown` here (rather than the actual React types) so this
 * module stays usable from non-React contexts — `studio.config.ts`
 * doesn't import React. The web shell narrows these to the right
 * function signatures at registration time.
 */
export interface StudioCellRendererProps {
  value: unknown;
  row: Record<string, unknown>;
  column: ColumnConfig;
}

export interface StudioPageProps {
  /** Manifest passed to every custom page so they can introspect entities. */
  manifest: unknown;
  /** Auth context — `me`, `is_admin`, etc. */
  auth: unknown;
  /** Imperative `api()` helper, same one built-in pages use. */
  api: <T = unknown>(path: string, opts?: unknown) => Promise<T>;
}

/** Registry shape — what `studio.entry.tsx` exports / registers. */
export interface StudioExtensions {
  renderers?: Record<string, unknown /* (p: StudioCellRendererProps) => ReactNode */>;
  pages?: Record<string, unknown /* (p: StudioPageProps) => ReactNode */>;
  layouts?: {
    footer?: unknown /* () => ReactNode */;
    headerActions?: unknown /* () => ReactNode */;
  };
}

/**
 * Identity helper for `studio.entry.tsx`. The user passes their component
 * map and exports the result as default. The Studio shell imports the
 * bundle and reads either `default` or the `registerStudioExtensions`
 * global, whichever the user used.
 *
 * ```tsx
 * // studio.entry.tsx
 * import { defineStudioExtensions } from "@pylon/sdk";
 * import { CurrencyCell } from "./renderers/currency";
 * import { Dashboard } from "./pages/dashboard";
 *
 * export default defineStudioExtensions({
 *   renderers: { currency: CurrencyCell },
 *   pages: { dashboard: Dashboard },
 * });
 * ```
 */
export function defineStudioExtensions(ext: StudioExtensions): StudioExtensions {
  return ext;
}
