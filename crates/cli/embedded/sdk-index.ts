// ---------------------------------------------------------------------------
// Route modes
// ---------------------------------------------------------------------------

export type RouteMode = "static" | "server" | "live" | "ssr";

// ---------------------------------------------------------------------------
// Field types
// ---------------------------------------------------------------------------

export type FieldType =
  | "string"
  | "int"
  | "float"
  | "bool"
  | "datetime"
  | "richtext"
  | `id(${string})`;

// ---------------------------------------------------------------------------
// Field builder
// ---------------------------------------------------------------------------

/**
 * CRDT container override for a field. Wire format is the kebab-case
 * string each variant maps to (`"text"`, `"counter"`, `"movable-list"`,
 * etc.). Mirror of `pylon_kernel::CrdtAnnotation` on the Rust side.
 *
 * - `"text"` upgrades a `string` to LoroText (collaborative
 *   character-level merge instead of LWW).
 * - `"counter"` flips an `int` / `float` to LoroCounter so concurrent
 *   increments add instead of stomping each other.
 * - `"list"`, `"movable-list"`, `"tree"` are reserved for ordered /
 *   reorderable / hierarchical collections — wire format locked in,
 *   server-side projection still pending implementation.
 * - `"lww"` is explicit (matches the default for most scalar types).
 */
export type CrdtAnnotation =
  | "lww"
  | "text"
  | "counter"
  | "list"
  | "movable-list"
  | "tree";

export interface FieldDefinition {
  type: FieldType;
  optional: boolean;
  unique: boolean;
  /** CRDT container override. Omitted entirely for the default
   *  (LWW for scalars, LoroText for richtext). */
  crdt?: CrdtAnnotation;
}

interface FieldBuilder {
  readonly _def: FieldDefinition;
  optional(): FieldBuilder;
  unique(): FieldBuilder;
  /**
   * Override the CRDT container for this field. See [`CrdtAnnotation`]
   * for the full list. Most apps never call this — the default mapping
   * (string→LWW, richtext→LoroText, …) is the right answer.
   *
   * Example: `field.string().crdt("text")` upgrades a string to a
   * collaborative LoroText so two browser tabs editing the field
   * concurrently merge cleanly instead of last-write-wins.
   */
  crdt(annotation: CrdtAnnotation): FieldBuilder;
}

function createFieldBuilder(type: FieldType): FieldBuilder {
  return buildField({ type, optional: false, unique: false });
}

function buildField(def: FieldDefinition): FieldBuilder {
  return {
    _def: def,
    optional() {
      return buildField({ ...def, optional: true });
    },
    unique() {
      return buildField({ ...def, unique: true });
    },
    crdt(annotation) {
      return buildField({ ...def, crdt: annotation });
    },
  };
}

// Both naming conventions ("bool"/"boolean", "float"/"number") are
// accepted here to match the validator side (`@pylonsync/functions`,
// where `v.bool/v.boolean` and `v.float/v.number` are aliases). Keeping
// both forms alive eliminates a real class of 'module fails to load'
// bugs caused by guessing which camp the API falls into.
export const field = {
  string: () => createFieldBuilder("string"),
  int: () => createFieldBuilder("int"),
  float: () => createFieldBuilder("float"),
  /** Alias for `field.float()`. Lets either name work. */
  number: () => createFieldBuilder("float"),
  bool: () => createFieldBuilder("bool"),
  /** Alias for `field.bool()`. Lets either name work. */
  boolean: () => createFieldBuilder("bool"),
  datetime: () => createFieldBuilder("datetime"),
  richtext: () => createFieldBuilder("richtext"),
  id: (target: string) => createFieldBuilder(`id(${target})`),
};

// ---------------------------------------------------------------------------
// Entity builder
// ---------------------------------------------------------------------------

export interface IndexDefinition {
  name: string;
  fields: string[];
  unique: boolean;
  /**
   * Optional SQL predicate. When set, the framework emits a *partial*
   * index — `CREATE [UNIQUE] INDEX … WHERE <predicate>` — so the index
   * (and any uniqueness constraint) only applies to rows matching the
   * predicate.
   *
   * Use case: enforce "max 1 hobby-tier org per user" without breaking
   * paid users who legitimately own many orgs:
   *
   * ```ts
   * indexes: [{
   *   name: "uniq_hobby_owner",
   *   fields: ["createdBy"],
   *   unique: true,
   *   where: "plan = 'hobby'",
   * }]
   * ```
   *
   * The predicate is passed straight through to the database. Both
   * SQLite and Postgres accept this syntax — write SQL the underlying
   * engine understands. Pylon does NOT validate or escape this string,
   * so DO NOT interpolate user input here.
   */
  where?: string;
}

export interface RelationDefinition {
  name: string;
  target: string;
  field: string;
  many?: boolean;
}

/**
 * Per-entity search config. Presence of this object on an entity
 * definition tells Pylon to create FTS5 + facet-bitmap shadow tables
 * on the next schema push and maintain them on every write.
 *
 * - `text`     – fields that participate in free-text MATCH (BM25).
 * - `facets`   – scalar fields (string / int / bool) that get live
 *                per-value counts via `db.useSearch`.
 * - `sortable` – fields the client may order results by. Any `sort`
 *                on a field not in this list is silently ignored.
 */
export interface SearchConfig {
  text?: string[];
  facets?: string[];
  sortable?: string[];
}

export interface EntityDefinition {
  name: string;
  fields: Record<string, FieldBuilder>;
  indexes?: IndexDefinition[];
  relations?: RelationDefinition[];
  search?: SearchConfig;
}

export function entity(
  name: string,
  fields: Record<string, FieldBuilder>,
  options?: {
    indexes?: IndexDefinition[];
    relations?: RelationDefinition[];
    search?: SearchConfig;
  },
): EntityDefinition {
  return {
    name,
    fields,
    indexes: options?.indexes,
    relations: options?.relations,
    search: options?.search,
  };
}

export function relation(def: RelationDefinition): RelationDefinition {
  return def;
}

// ---------------------------------------------------------------------------
// Route definition
// ---------------------------------------------------------------------------

export type AuthMode = "public" | "user";

export interface RouteDefinition {
  path: string;
  mode: RouteMode;
  query?: string;
  auth?: AuthMode;
  /**
   * Project-relative module path (e.g. `app/hello/page`) for SSR
   * routes. Required when `mode === "ssr"`. Discovered automatically
   * by `discoverAppRoutes()`; only specify manually for one-off
   * SSR routes outside the `app/` tree.
   */
  component?: string;
  /**
   * Layout module path chain (root→leaf). Each layout wraps the
   * next as `children`. Only relevant for `mode === "ssr"`.
   */
  layouts?: string[];
}

export function defineRoute(route: RouteDefinition): RouteDefinition {
  return route;
}

// ---------------------------------------------------------------------------
// Query definition
// ---------------------------------------------------------------------------

export interface InputFieldDefinition {
  name: string;
  type: FieldType;
  optional?: boolean;
}

export interface QueryDefinition {
  name: string;
  input?: InputFieldDefinition[];
}

export function query(
  name: string,
  options?: { input?: InputFieldDefinition[] }
): QueryDefinition {
  return { name, input: options?.input };
}

// ---------------------------------------------------------------------------
// Action definition
// ---------------------------------------------------------------------------

export interface ActionDefinition {
  name: string;
  input?: InputFieldDefinition[];
}

export function action(
  name: string,
  options?: { input?: InputFieldDefinition[] }
): ActionDefinition {
  return { name, input: options?.input };
}

// ---------------------------------------------------------------------------
// Policy definition
// ---------------------------------------------------------------------------

export interface PolicyDefinition {
  name: string;
  entity?: string;
  action?: string;
  /**
   * Fallback allow expression — evaluated when a more-specific
   * allowRead/allowWrite/allowUpdate/allowDelete isn't set. Kept for
   * backwards compatibility with single-gate policies.
   */
  allow?: string;
  /** Overrides `allow` for reads (pull, list, get). */
  allowRead?: string;
  /** Overrides `allow` for inserts. Falls back to `allowWrite`. */
  allowInsert?: string;
  /** Overrides `allow`/`allowWrite` for updates. */
  allowUpdate?: string;
  /** Overrides `allow`/`allowWrite` for deletes. */
  allowDelete?: string;
  /** Shared fallback for any write when the specific rule is missing. */
  allowWrite?: string;
}

export function policy(def: PolicyDefinition): PolicyDefinition {
  return def;
}

// ---------------------------------------------------------------------------
// Plugin definition
// ---------------------------------------------------------------------------

export interface PluginDefinition {
  name: string;
  entities?: EntityDefinition[];
  hooks?: {
    beforeInsert?: (entity: string, data: Record<string, unknown>) => Record<string, unknown> | null;
    afterInsert?: (entity: string, id: string, data: Record<string, unknown>) => void;
    beforeUpdate?: (entity: string, id: string, data: Record<string, unknown>) => Record<string, unknown> | null;
    afterUpdate?: (entity: string, id: string, data: Record<string, unknown>) => void;
    beforeDelete?: (entity: string, id: string) => boolean;
    afterDelete?: (entity: string, id: string) => void;
  };
}

export function definePlugin(def: PluginDefinition): PluginDefinition {
  return def;
}

// ---------------------------------------------------------------------------
// Manifest generation
// ---------------------------------------------------------------------------

export interface ManifestField {
  name: string;
  type: FieldType;
  optional: boolean;
  unique: boolean;
  /** CRDT container override; matches `pylon_kernel::CrdtAnnotation` on
   *  the Rust side. Omitted entirely when the field uses the default. */
  crdt?: CrdtAnnotation;
}

export interface ManifestIndex {
  name: string;
  fields: string[];
  unique: boolean;
  /** Optional partial-index predicate — see `IndexDefinition.where`. */
  where?: string;
}

export interface ManifestRelation {
  name: string;
  target: string;
  field: string;
  many?: boolean;
}

export interface ManifestEntity {
  name: string;
  fields: ManifestField[];
  indexes: ManifestIndex[];
  relations?: ManifestRelation[];
  /**
   * Mirrors `pylon_kernel::ManifestSearchConfig`. When present, the
   * runtime creates FTS5 + facet-bitmap shadow tables on schema push
   * and maintains them on every write.
   */
  search?: {
    text?: string[];
    facets?: string[];
    sortable?: string[];
  };
}

export interface ManifestRoute {
  path: string;
  mode: string;
  query?: string;
  auth?: string;
  /**
   * Project-relative module path (e.g. `app/hello/page`) for
   * file-based SSR routes. Populated by the discoverer; absent
   * for legacy mode:"static" / "server" / "live" routes.
   */
  component?: string;
  /**
   * Layout chain walked root→leaf. Each entry is a project-
   * relative module path. Empty when no layouts apply.
   */
  layouts?: string[];
}

export interface ManifestInputField {
  name: string;
  type: FieldType;
  optional: boolean;
  unique: false;
}

export interface ManifestQuery {
  name: string;
  input?: ManifestInputField[];
}

export interface ManifestAction {
  name: string;
  input?: ManifestInputField[];
}

export interface ManifestPolicy {
  name: string;
  entity?: string;
  action?: string;
  allow?: string;
  allowRead?: string;
  allowInsert?: string;
  allowUpdate?: string;
  allowDelete?: string;
  allowWrite?: string;
}

export const MANIFEST_VERSION = 1;

export interface AppManifest {
  manifest_version: number;
  name: string;
  version: string;
  entities: ManifestEntity[];
  routes: ManifestRoute[];
  queries: ManifestQuery[];
  actions: ManifestAction[];
  policies: ManifestPolicy[];
  auth?: ManifestAuthConfig;
}

export function entitiesToManifest(
  entities: EntityDefinition[]
): ManifestEntity[] {
  return entities.map((e) => {
    const result: ManifestEntity = {
      name: e.name,
      fields: Object.entries(e.fields).map(([name, fb]) => {
        const f: ManifestField = {
          name,
          type: fb._def.type,
          optional: fb._def.optional,
          unique: fb._def.unique,
        };
        // Emit `crdt` only when set — keeps default-shape manifests
        // visually identical to pre-CRDT versions in JSON diffs.
        if (fb._def.crdt !== undefined) {
          f.crdt = fb._def.crdt;
        }
        return f;
      }),
      indexes: (e.indexes ?? []).map((idx) => ({
        name: idx.name,
        fields: idx.fields,
        unique: idx.unique,
        ...(idx.where ? { where: idx.where } : {}),
      })),
    };
    if (e.relations && e.relations.length > 0) {
      result.relations = e.relations.map((r) => ({
        name: r.name,
        target: r.target,
        field: r.field,
        many: r.many,
      }));
    }
    if (e.search) {
      const s = e.search;
      // Only emit the block when at least one list is non-empty — keeps
      // the manifest JSON clean for non-searchable entities.
      const anyDeclared =
        (s.text?.length ?? 0) > 0 ||
        (s.facets?.length ?? 0) > 0 ||
        (s.sortable?.length ?? 0) > 0;
      if (anyDeclared) {
        result.search = {
          text: s.text ?? [],
          facets: s.facets ?? [],
          sortable: s.sortable ?? [],
        };
      }
    }
    return result;
  });
}

export function routesToManifest(routes: RouteDefinition[]): ManifestRoute[] {
  return routes.map((r) => {
    const result: ManifestRoute = { path: r.path, mode: r.mode };
    if (r.query) result.query = r.query;
    if (r.auth) result.auth = r.auth;
    if (r.component) result.component = r.component;
    if (r.layouts && r.layouts.length > 0) result.layouts = r.layouts;
    return result;
  });
}

/**
 * Walk the project's `app/` directory and discover file-based SSR
 * routes. Returns a list of `RouteDefinition` ready to slot into
 * `buildManifest({ routes })`.
 *
 * Mapping rules (Next App Router-shaped):
 *   - `app/page.tsx` → `/`
 *   - `app/about/page.tsx` → `/about`
 *   - `app/blog/[slug]/page.tsx` → `/blog/:slug`
 *   - `app/layout.tsx` wraps every page below; `app/blog/layout.tsx`
 *     wraps `/blog/*`.
 *
 * All discovered routes are `mode: "ssr"`. Phase 1 doesn't yet
 * support `loading.tsx` / `error.tsx` / `not-found.tsx` — those
 * surface as warnings in `pylon lint` (Phase 2 wires them).
 *
 * Pure Node `readdirSync` walk — no glob dep. Sorts deterministically:
 * literal segments before parameterized ones at each depth, so the
 * Rust matcher's first-match-wins lookup picks the right route.
 *
 * `appDir` defaults to `<cwd>/app`. Pass an absolute path to scan a
 * different layout (monorepo subprojects, etc.).
 */
export function discoverAppRoutes(opts?: {
  appDir?: string;
}): RouteDefinition[] {
  // Pull `fs` + `path` lazily so users without an `app/` dir don't
  // pay the import cost (and so this file stays browser-loadable
  // for any future codegen client that imports the same module).
  // Bun resolves these synchronously from the user-process side.
  const fs: typeof import("node:fs") = require("node:fs");
  const path: typeof import("node:path") = require("node:path");

  const cwd = process.cwd();
  const appDir =
    opts?.appDir && path.isAbsolute(opts.appDir)
      ? opts.appDir
      : path.join(cwd, opts?.appDir ?? "app");
  if (!fs.existsSync(appDir) || !fs.statSync(appDir).isDirectory()) {
    return [];
  }

  type PageHit = {
    /** Relative slugs from app root (e.g. ["blog", "[slug]"]). */
    segments: string[];
    /** Project-relative module path WITHOUT extension. */
    component: string;
    /** Project-relative layout chain (closest-root-first). */
    layouts: string[];
  };

  const pages: PageHit[] = [];

  function walk(dir: string, segments: string[], layouts: string[]): void {
    let entries: import("node:fs").Dirent[];
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    // Layout file at THIS depth (applied to every page beneath).
    const layoutHere = ["layout.tsx", "layout.ts", "layout.jsx", "layout.js"]
      .map((n) => path.join(dir, n))
      .find((p) => fs.existsSync(p));
    const nextLayouts = layoutHere
      ? [...layouts, path.relative(cwd, layoutHere).replace(/\.(tsx?|jsx?)$/, "")]
      : layouts;
    // Page file at THIS depth.
    const pageHere = ["page.tsx", "page.ts", "page.jsx", "page.js"]
      .map((n) => path.join(dir, n))
      .find((p) => fs.existsSync(p));
    if (pageHere) {
      pages.push({
        segments: [...segments],
        component: path.relative(cwd, pageHere).replace(/\.(tsx?|jsx?)$/, ""),
        layouts: nextLayouts,
      });
    }
    // Recurse into subdirs.
    for (const e of entries) {
      if (!e.isDirectory()) continue;
      // Skip dotfiles, node_modules, and Next-style private "(group)"
      // segments don't affect the URL path — strip them out so
      // `app/(marketing)/about/page.tsx` resolves to `/about`.
      if (e.name.startsWith(".") || e.name === "node_modules") continue;
      const sub = path.join(dir, e.name);
      const isGroup = e.name.startsWith("(") && e.name.endsWith(")");
      const newSegments = isGroup ? segments : [...segments, e.name];
      walk(sub, newSegments, nextLayouts);
    }
  }

  walk(appDir, [], []);

  // Sort: literal segments before parameterized ones at each depth.
  // Otherwise `/blog/:slug` could shadow `/blog/featured` depending
  // on FS order.
  const isParam = (s: string): boolean =>
    s.startsWith("[") && s.endsWith("]");
  pages.sort((a, b) => {
    const minLen = Math.min(a.segments.length, b.segments.length);
    for (let i = 0; i < minLen; i++) {
      const ap = isParam(a.segments[i]);
      const bp = isParam(b.segments[i]);
      if (ap !== bp) return ap ? 1 : -1;
      if (a.segments[i] !== b.segments[i]) {
        return a.segments[i] < b.segments[i] ? -1 : 1;
      }
    }
    return a.segments.length - b.segments.length;
  });

  return pages.map((p) => ({
    path:
      "/" +
      p.segments
        .map((s) =>
          isParam(s) ? `:${s.slice(1, -1)}` : s,
        )
        .join("/"),
    mode: "ssr" as const,
    component: p.component,
    layouts: p.layouts,
  }));
}

export function queriesToManifest(queries: QueryDefinition[]): ManifestQuery[] {
  return queries.map((q) => {
    const result: ManifestQuery = { name: q.name };
    if (q.input && q.input.length > 0) {
      result.input = q.input.map((f) => ({
        name: f.name,
        type: f.type,
        optional: f.optional ?? false,
        unique: false as const,
      }));
    }
    return result;
  });
}

export function actionsToManifest(
  actions: ActionDefinition[]
): ManifestAction[] {
  return actions.map((a) => {
    const result: ManifestAction = { name: a.name };
    if (a.input && a.input.length > 0) {
      result.input = a.input.map((f) => ({
        name: f.name,
        type: f.type,
        optional: f.optional ?? false,
        unique: false as const,
      }));
    }
    return result;
  });
}

export function policiesToManifest(
  policies: PolicyDefinition[]
): ManifestPolicy[] {
  return policies.map((p) => {
    const result: ManifestPolicy = { name: p.name };
    if (p.allow) result.allow = p.allow;
    if (p.allowRead) result.allowRead = p.allowRead;
    if (p.allowInsert) result.allowInsert = p.allowInsert;
    if (p.allowUpdate) result.allowUpdate = p.allowUpdate;
    if (p.allowDelete) result.allowDelete = p.allowDelete;
    if (p.allowWrite) result.allowWrite = p.allowWrite;
    if (p.entity) result.entity = p.entity;
    if (p.action) result.action = p.action;
    return result;
  });
}

/**
 * Auth configuration block for the manifest. Mirrors better-auth's
 * `betterAuth({ user, session, trustedOrigins })` shape.
 *
 * All fields optional with sensible defaults — apps that don't pass
 * an `auth({...})` block to `buildManifest` get the framework defaults
 * (User entity named "User", strip `passwordHash`, 30-day sessions,
 * no cookie cache, trusted origins from `PYLON_TRUSTED_ORIGINS` env).
 *
 * `trustedOrigins` is the unified source for **all three gates** —
 * CORS, CSRF, and OAuth-redirect. Loopback origins
 * (`http://localhost`, `127.0.0.1`, `[::1]`, any port) are always
 * auto-trusted across all three gates so `pylon dev` works without
 * any allowlist config.
 *
 * @example
 * auth({
 *   user: {
 *     entity: "User",
 *     expose: ["id", "email", "displayName"],
 *     hide: ["passwordHash", "internalNotes"],
 *   },
 *   session: { expiresIn: 60 * 60 * 24 * 7 }, // 7 days
 *   trustedOrigins: ["https://app.example.com"],
 * })
 */
export type AuthConfig = {
  user?: {
    /** Manifest entity name pylon treats as the User table. Default `"User"`. */
    entity?: string;
    /** Allowlist of fields exposed via `/api/auth/session`. Empty = all (minus hide list). */
    expose?: string[];
    /** Additional fields stripped (combined with default `passwordHash` + `_*`). */
    hide?: string[];
    /**
     * Field on the User row that, when truthy, lifts the session's
     * `auth.is_admin = true`. Examples: `"isAdmin"` (bool column),
     * `"role"` (string equal to "admin"), `"roles"` (array containing
     * "admin"). Default unset — only `PYLON_ADMIN_TOKEN` grants admin.
     *
     * Set this when you want platform admins to sign in with their
     * regular account (Studio gates on `is_admin`, dashboards can
     * branch on it, etc.). The env-token path keeps working as the
     * bootstrap / CI escape hatch.
     */
    adminField?: string;
  };
  session?: {
    /** New session lifetime in seconds. Default 30 days. */
    expiresIn?: number;
    /** Cookie cache config — bake claims into the cookie so reads avoid the DB. */
    cookieCache?: {
      enabled?: boolean;
      /** Max staleness in seconds. Default 5 minutes. */
      maxAge?: number;
      /** Auth-context fields baked into the cookie envelope (always includes `user_id`). */
      claims?: string[];
    };
  };
  /**
   * Org / OrgMember / OrgInvite entity configuration. Apps that use
   * the framework's `/api/auth/orgs/*` surface declare these entities
   * in their schema with the framework's required fields. Add custom
   * fields freely (logo, industry, billingEmail, etc.) — the framework
   * reads / writes only the fields it manages.
   *
   * Defaults to entities named `Org`, `OrgMember`, `OrgInvite`. Rename
   * via the three string fields if your codebase uses different names
   * (e.g. `Organization` / `Membership`). Set `disabled: true` to opt
   * out of the framework's routes entirely — useful when the app has
   * its own org flow in TS and doesn't want the framework's parallel
   * write paths.
   */
  org?: {
    /** Entity name for the org table. Default `"Org"`. */
    entity?: string;
    /** Entity name for membership rows. Default `"OrgMember"`. */
    memberEntity?: string;
    /** Entity name for invite rows. Default `"OrgInvite"`. */
    inviteEntity?: string;
    /**
     * Disable the framework's `/api/auth/orgs/*` routes. Endpoints
     * return `501 ORG_NOT_CONFIGURED`. Use when you implement org
     * management entirely in your own TypeScript functions.
     */
    disabled?: boolean;
  };
  /**
   * Per-app trusted origins. Single declarative source for the three
   * browser-facing gates: CORS, CSRF, OAuth `?callback=` validation.
   * Merged with `PYLON_TRUSTED_ORIGINS` (OAuth) / `PYLON_CORS_ORIGIN`
   * (CORS) / `PYLON_CSRF_ORIGINS` (CSRF) env vars when ops need to
   * split per-gate. Loopback (`http://localhost`, `127.0.0.1`, `[::1]`,
   * any port) is always auto-trusted at every gate.
   */
  trustedOrigins?: string[];
};

export type ManifestAuthConfig = {
  user: {
    entity: string;
    expose: string[];
    hide: string[];
    admin_field?: string;
  };
  session: {
    expires_in: number;
    cookie_cache: {
      enabled: boolean;
      max_age: number;
      claims: string[];
    };
  };
  org: {
    entity: string;
    member_entity: string;
    invite_entity: string;
    disabled: boolean;
  };
  trusted_origins: string[];
};

/**
 * Build the manifest's `auth` block from the user-facing camelCase
 * config. Translates to the snake_case shape the Rust runtime expects.
 *
 * Defaults match `pylon_kernel::ManifestAuthConfig::default()` so
 * passing nothing is equivalent to omitting the `auth({...})` call.
 */
export function auth(cfg: AuthConfig = {}): ManifestAuthConfig {
  return {
    user: {
      entity: cfg.user?.entity ?? "User",
      expose: cfg.user?.expose ?? [],
      hide: cfg.user?.hide ?? [],
      ...(cfg.user?.adminField ? { admin_field: cfg.user.adminField } : {}),
    },
    session: {
      expires_in: cfg.session?.expiresIn ?? 30 * 24 * 60 * 60,
      cookie_cache: {
        enabled: cfg.session?.cookieCache?.enabled ?? false,
        max_age: cfg.session?.cookieCache?.maxAge ?? 5 * 60,
        claims: cfg.session?.cookieCache?.claims ?? ["is_admin", "tenant_id"],
      },
    },
    org: {
      entity: cfg.org?.entity ?? "Org",
      member_entity: cfg.org?.memberEntity ?? "OrgMember",
      invite_entity: cfg.org?.inviteEntity ?? "OrgInvite",
      disabled: cfg.org?.disabled ?? false,
    },
    trusted_origins: cfg.trustedOrigins ?? [],
  };
}

export function buildManifest(options: {
  name: string;
  version: string;
  entities: EntityDefinition[];
  routes: RouteDefinition[];
  queries?: QueryDefinition[];
  actions?: ActionDefinition[];
  policies?: PolicyDefinition[];
  auth?: ManifestAuthConfig;
}): AppManifest {
  return {
    manifest_version: MANIFEST_VERSION,
    name: options.name,
    version: options.version,
    entities: entitiesToManifest(options.entities),
    routes: routesToManifest(options.routes),
    queries: queriesToManifest(options.queries ?? []),
    actions: actionsToManifest(options.actions ?? []),
    policies: policiesToManifest(options.policies ?? []),
    auth: options.auth ?? auth(),
  };
}

// ---------------------------------------------------------------------------
// Studio configuration — re-exports
// ---------------------------------------------------------------------------

export {
  defineStudioConfig,
  defineStudioExtensions,
  type BrandConfig,
  type ThemeConfig,
  type ThemeAccent,
  type ThemeAppearance,
  type IconName,
  type SidebarConfig,
  type SidebarSection,
  type SidebarItem,
  type SidebarPageItem,
  type SidebarResourceItem,
  type SidebarLinkItem,
  type SidebarHeadingItem,
  type SidebarFooter,
  type SidebarFooterCard,
  type SidebarFooterCustom,
  type ResourceConfig,
  type ResourceListConfig,
  type ColumnConfig,
  type ColumnRenderer,
  type RendererKind,
  type RendererText,
  type RendererAvatar,
  type RendererBadge,
  type RendererDate,
  type RendererLink,
  type RendererBoolean,
  type RendererNumber,
  type RendererJson,
  type RendererCustom,
  type BulkAction,
  type RowAction,
  type PageConfig,
  type StudioConfig,
  type StudioCellRendererProps,
  type StudioPageProps,
  type StudioExtensions,
} from "./studio";
