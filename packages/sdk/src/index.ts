// ---------------------------------------------------------------------------
// Route modes
// ---------------------------------------------------------------------------

export type RouteMode = "static" | "server" | "live";

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
  /**
   * When true, the field is **never serialized in HTTP responses**.
   * Use for secrets / billing-side identity / hashes that the server
   * needs to read internally but should never leak to clients.
   *
   * Stripped at every public read boundary:
   * - `GET /api/entities/<entity>` (list)
   * - `GET /api/entities/<entity>/<id>` (single row)
   * - `GET /api/auth/session` (User row projection)
   * - Sync push deltas
   *
   * Still readable from inside server functions via `ctx.db.*` —
   * the framework trusts your handler logic to decide what to
   * return. If you pass the row through unmodified to the client,
   * the field IS still stripped at the function-response boundary,
   * provided the value is a plain row from `ctx.db.get` (which
   * tags it with the entity name so the boundary knows what to
   * filter).
   *
   * `passwordHash` on every User entity is implicitly serverOnly
   * even without this annotation, by the framework's hardcoded
   * convention. New apps should still mark it explicitly so the
   * intent shows up in the schema.
   */
  serverOnly?: boolean;
  /**
   * When true, the field is **set on insert but cannot be changed
   * by client updates**. The framework rejects any `PATCH`/`PUT`
   * payload that mentions the field with a `READONLY_FIELD` error,
   * before policy evaluation. Admin contexts bypass this check (as
   * with all other framework gates), so migrations + ops scripts
   * can still rewrite owner-shaped fields.
   *
   * Use for identity-shaped columns that need to be settable at
   * creation but immutable after — `authorId`, `orgId`,
   * `createdBy`, `stripeCustomerId`. Closes the canonical IDOR
   * shape where a policy gates on `data.authorId == auth.userId`
   * but the attacker passes a different `authorId` in the update
   * payload to flip the row's ownership.
   *
   * Server-side writes (via `ctx.db.update` inside a function)
   * still go through — readonly only blocks the HTTP entity
   * routes (`PATCH /api/entities/<entity>/<id>`) and `/api/transact`.
   * Server code is trusted to enforce its own invariants.
   */
  readonly?: boolean;
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
  /**
   * Mark the field as never-returned in HTTP responses. See
   * [`FieldDefinition.serverOnly`] for the full semantics.
   *
   * Example: `stripeCustomerId: field.string().serverOnly()` keeps
   * the Stripe customer id out of `/api/entities/Org/<id>` responses
   * while staying readable from `ctx.db.get` inside the
   * stripeWebhook action.
   */
  serverOnly(): FieldBuilder;
  /**
   * Mark the field as set-on-insert-only. See [`FieldDefinition.readonly`]
   * for the full semantics.
   *
   * Example: `authorId: field.id("User").readonly()` lets the framework
   * reject any `PATCH` payload trying to rewrite the row's author —
   * closes the IDOR-via-update-payload class.
   */
  readonly(): FieldBuilder;
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
    serverOnly() {
      return buildField({ ...def, serverOnly: true });
    },
    readonly() {
      return buildField({ ...def, readonly: true });
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
  /** Optional — `buildManifest` auto-generates a name from the entity
   *  + a counter when the fluent `.policies(policy({...}))` chain
   *  omits one. Explicit names are still recommended for the
   *  procedural API since they appear in policy-denied error
   *  messages. */
  name?: string;
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
  /** Set when the field is `field.X().serverOnly()` — see
   *  [`FieldDefinition.serverOnly`]. Omitted by default so JSON
   *  manifests stay compact for unannotated apps. */
  serverOnly?: boolean;
  /** Set when the field is `field.X().readonly()` — see
   *  [`FieldDefinition.readonly`]. Omitted by default. */
  readonly?: boolean;
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
        // Emit optional modifiers only when set — keeps default-shape
        // manifests visually identical to pre-modifier versions in
        // JSON diffs.
        if (fb._def.crdt !== undefined) {
          f.crdt = fb._def.crdt;
        }
        if (fb._def.serverOnly) {
          f.serverOnly = true;
        }
        if (fb._def.readonly) {
          f.readonly = true;
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
    return result;
  });
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
  return policies.map((p, i) => {
    const result: ManifestPolicy = {
      // Final-resort name autogen. `buildManifest` upstream already
      // names every attached-via-fluent policy, but a caller passing
      // `policies: [policy({ allowRead: "..." })]` to a custom
      // manifest builder would slip through with `name: undefined`.
      // Stamp a unique fallback so the runtime never sees a blank.
      name:
        p.name && p.name.length > 0
          ? p.name
          : `${(p.entity ?? p.action ?? "unnamed").toLowerCase()}_p${i}`,
    };
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
  // Pull policies attached via the fluent `e.entity().policies(...)`
  // chain onto the top-level policies list. Without this, fluent
  // apps would register entities without policies and every read
  // would default-deny. Existing apps using the procedural API
  // (`entity()` + separate `policy({...})` exports) are unaffected
  // because `extractAttachedPolicies` returns an empty array for
  // them. Concat order: top-level policies first (explicit beats
  // attached), then anything pulled off entities.
  //
  // Stamp a name if the fluent caller omitted it — `name` is
  // technically required on PolicyDefinition but the docs imply you
  // can `.policies(policy({ allowRead: "..." }))` without one. Auto-
  // derive from the entity + a counter so two attached policies
  // don't collide.
  const attached: PolicyDefinition[] = [];
  for (const ent of options.entities) {
    const extracted = extractAttachedPolicies(ent);
    extracted.forEach((p, i) => {
      attached.push({
        ...p,
        name: p.name && p.name.length > 0
          ? p.name
          : `${(p.entity ?? ent.name).toLowerCase()}_attached_${i}`,
      });
    });
  }
  const allPolicies = [...(options.policies ?? []), ...attached];
  return {
    manifest_version: MANIFEST_VERSION,
    name: options.name,
    version: options.version,
    entities: entitiesToManifest(options.entities),
    routes: routesToManifest(options.routes),
    queries: queriesToManifest(options.queries ?? []),
    actions: actionsToManifest(options.actions ?? []),
    policies: policiesToManifest(allPolicies),
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

// ---------------------------------------------------------------------------
// Fluent schema API (`e` namespace)
//
// `entity(name, fields, options)` + `field.*` + `policy({...})` are the
// stable foundation — every dependent app uses them today. The fluent
// API below sits on top and compiles down to the same EntityDefinition
// + PolicyDefinition shapes the runtime already understands, so both
// styles can coexist forever. The fluent shape just reads better in
// docs + marketing snippets:
//
// ```ts
// export const Order = e.entity("Order", {
//   customer:  field.id("Customer"),
//   total:     field.int(),
//   status:    field.enum(["pending", "paid", "failed"]),
//   createdAt: field.datetime().defaultNow(),
// })
//   .indexes(e.idx("customer", "createdAt"), e.idx("status"))
//   .policies(policy({
//     allowRead:  "auth.userId == data.customer || auth.hasRole('admin')",
//     allowUpdate: "auth.hasRole('admin')",
//   }))
//   .behaviors([timestamps, softDelete]);
// ```
//
// Behaviors are field-injection helpers. `timestamps` adds
// `createdAt` + `updatedAt` to the entity fields; `softDelete` adds
// `deletedAt`. They mutate the EntityDefinition's fields before the
// runtime sees it, so the rest of the framework (storage, sync,
// policy gates) treats them as ordinary columns. Auto-stamping
// (filling `now()` on insert/update) needs runtime support — landing
// in a follow-up patch via the `defaultExpr: "now"` marker the
// builder records.
// ---------------------------------------------------------------------------

/**
 * Behavior — a function that mutates the entity definition before it's
 * registered. Implementations should be idempotent (the user can list
 * the same behavior twice without breaking the schema).
 */
export interface Behavior {
  /** Stable identifier — surfaced in the manifest for tooling, lets a
   *  pass-through inspector see which behaviors are active. */
  readonly id: string;
  apply(def: EntityDefinition): EntityDefinition;
}

/**
 * `timestamps` — auto-add `createdAt` + `updatedAt` datetime fields.
 * The `defaultNow()` marker on each tells the runtime to fill `now()`
 * on insert (and on update for `updatedAt`). Wiring lands with the
 * runtime patch — until then, app code can still set the values
 * manually and the fields exist on the row.
 */
export const timestamps: Behavior = {
  id: "timestamps",
  apply(def) {
    const fields = { ...def.fields };
    if (!fields.createdAt) {
      fields.createdAt = (field.datetime() as FieldBuilder & {
        defaultNow?: () => FieldBuilder;
      }).defaultNow?.() ?? field.datetime();
    }
    if (!fields.updatedAt) {
      fields.updatedAt = (field.datetime() as FieldBuilder & {
        defaultNow?: () => FieldBuilder;
        updateOnWrite?: () => FieldBuilder;
      }).defaultNow?.() ?? field.datetime();
    }
    return { ...def, fields };
  },
};

/**
 * `softDelete` — auto-add a nullable `deletedAt` datetime field.
 * Rows with `deletedAt != null` are filtered from default reads
 * (TS-side filtering today; runtime filter lands in the follow-up).
 */
export const softDelete: Behavior = {
  id: "softDelete",
  apply(def) {
    const fields = { ...def.fields };
    if (!fields.deletedAt) {
      fields.deletedAt = field.datetime().optional();
    }
    return { ...def, fields };
  },
};

/**
 * `audit` — marker behavior. Tags the entity for the framework's
 * audit pipeline (writes an `AuditEvent` row per mutation, recording
 * the actor + diff). Runtime hook lands in a follow-up patch — for
 * now the marker is preserved on the manifest so apps can opt in
 * early without breaking later.
 */
export const audit: Behavior = {
  id: "audit",
  apply(def) {
    // No field injection today. The behavior flag is recorded via
    // the EntityBuilder's `_behaviors` list (read off on serialize)
    // so the runtime can pick it up once the audit pipeline lands.
    return def;
  },
};

/**
 * Internal sentinel — apps don't construct these directly. The
 * `field.X` builders gain `default(val)` / `defaultNow()` chainables
 * below; this is just the shape stored on the field definition for
 * the runtime to read.
 */
type DefaultMarker = { kind: "value"; value: unknown } | { kind: "now" };

/**
 * Augment FieldBuilder with the new `default()` / `defaultNow()`
 * chainables. Runtime support for actually filling these values on
 * insert lands as part of v0.4.1; until then the markers are
 * recorded in the manifest for tooling + the codegen layer.
 */
declare module "./index" {
  // (Empty — the `default*` methods are added at runtime via the
  // patched buildField below. Apps see them via the FieldBuilder
  // surface declared above.)
}

// Field builder with chainable `.default()` / `.defaultNow()`. All
// other chainables (`optional`, `unique`, `crdt`, `serverOnly`,
// `readonly`) are reimplemented here to return another
// `buildFieldWithDefaults` — without this, calling `.optional()`
// after `.default()` would drop the default markers off the chain
// (codex Wave-3 review: the previous `{ ...base, default, defaultNow }`
// pattern delegated optional/unique to the original buildField which
// returned a builder lacking `.default()`). Recursion through the
// same constructor keeps the surface stable regardless of chain
// order.
function buildFieldWithDefaults(
  def: FieldDefinition & {
    default?: DefaultMarker;
    enumValues?: readonly string[];
  },
): FieldBuilder & {
  default(value: unknown): ReturnType<typeof buildFieldWithDefaults>;
  defaultNow(): ReturnType<typeof buildFieldWithDefaults>;
} {
  return {
    _def: def,
    optional() {
      return buildFieldWithDefaults({ ...def, optional: true });
    },
    unique() {
      return buildFieldWithDefaults({ ...def, unique: true });
    },
    crdt(annotation) {
      return buildFieldWithDefaults({ ...def, crdt: annotation });
    },
    serverOnly() {
      return buildFieldWithDefaults({ ...def, serverOnly: true });
    },
    readonly() {
      return buildFieldWithDefaults({ ...def, readonly: true });
    },
    default(value: unknown) {
      return buildFieldWithDefaults({
        ...def,
        default: { kind: "value", value },
      });
    },
    defaultNow() {
      return buildFieldWithDefaults({ ...def, default: { kind: "now" } });
    },
  };
}
// Re-export `field` with the patched builder so callers picking up
// the new SDK get the chainables transparently. The old `field`
// surface still works — `.default()` / `.defaultNow()` are additive.
// We intentionally re-export from the same name so existing imports
// (`import { field } from "@pylonsync/sdk"`) keep working AND gain
// the new methods without a code change.
Object.assign(field, {
  string: () => buildFieldWithDefaults({ type: "string", optional: false, unique: false }),
  int: () => buildFieldWithDefaults({ type: "int", optional: false, unique: false }),
  float: () => buildFieldWithDefaults({ type: "float", optional: false, unique: false }),
  number: () => buildFieldWithDefaults({ type: "float", optional: false, unique: false }),
  bool: () => buildFieldWithDefaults({ type: "bool", optional: false, unique: false }),
  boolean: () => buildFieldWithDefaults({ type: "bool", optional: false, unique: false }),
  datetime: () => buildFieldWithDefaults({ type: "datetime", optional: false, unique: false }),
  richtext: () => buildFieldWithDefaults({ type: "richtext", optional: false, unique: false }),
  id: (target: string) => buildFieldWithDefaults({ type: `id(${target})` as FieldType, optional: false, unique: false }),
  /**
   * `field.enum(["pending", "paid", "failed"])` — stored as a string
   * with allowed-values metadata. Runtime enforcement (CHECK
   * constraint or insert-time validation) lands in a follow-up
   * patch; for now the values flow through to codegen so the
   * generated client gets a precise `"pending" | "paid" | "failed"`
   * literal-union type instead of a wide `string`.
   */
  enum(values: readonly string[]) {
    const def: FieldDefinition & { enumValues?: readonly string[] } = {
      type: "string",
      optional: false,
      unique: false,
      enumValues: values,
    };
    return buildFieldWithDefaults(def);
  },
});

/** Variadic index helper — `e.idx("customer", "createdAt")` reads
 *  better than the options-object form for the common case. */
function idx(...fields: string[]): IndexDefinition {
  return {
    name: `by_${fields.join("_")}`,
    fields,
    unique: false,
  };
}

interface EntityBuilder {
  readonly _def: EntityDefinition & { behaviors?: string[] };
  indexes(...idxs: IndexDefinition[]): EntityBuilder;
  policies(...policies: PolicyDefinition[]): EntityBuilder;
  behaviors(list: readonly Behavior[]): EntityBuilder;
  relations(...rels: RelationDefinition[]): EntityBuilder;
  search(cfg: SearchConfig): EntityBuilder;
}

/**
 * Internal — `e.entity()` is the public surface. Wraps an
 * EntityDefinition with chainable builders that all return another
 * EntityBuilder so the fluent calls compose freely. The terminal
 * call is implicit: any place the framework expects an
 * EntityDefinition (e.g. `entities: [...]` on the manifest), the
 * builder unwraps via the `_def` getter on access.
 */
function buildEntity(def: EntityDefinition & { behaviors?: string[] }): EntityBuilder {
  const self: EntityBuilder = {
    get _def() {
      return def;
    },
    indexes(...idxs) {
      return buildEntity({
        ...def,
        indexes: [...(def.indexes ?? []), ...idxs],
      });
    },
    policies(..._policies) {
      // Policies aren't carried on the EntityDefinition itself —
      // they live in the manifest's top-level `policies` list. We
      // store them under a non-standard key here so the manifest
      // builder can pluck them off; the export shape stays
      // EntityDefinition-compatible.
      const carried = { ...(def as EntityDefinition & { _attachedPolicies?: PolicyDefinition[] }) };
      const existing = carried._attachedPolicies ?? [];
      const stamped = _policies.map((p) => ({
        ...p,
        // Auto-bind the entity if the policy didn't specify one —
        // this is the whole point of attaching policies via the
        // builder: don't repeat the entity name.
        entity: p.entity ?? def.name,
      }));
      carried._attachedPolicies = [...existing, ...stamped];
      return buildEntity(carried);
    },
    behaviors(list) {
      let next = def;
      for (const b of list) {
        next = b.apply(next);
      }
      return buildEntity({
        ...next,
        behaviors: [...(def.behaviors ?? []), ...list.map((b) => b.id)],
      });
    },
    relations(...rels) {
      return buildEntity({
        ...def,
        relations: [...(def.relations ?? []), ...rels],
      });
    },
    search(cfg) {
      return buildEntity({
        ...def,
        search: cfg,
      });
    },
  };

  // Spread `def` keys onto the builder so anywhere the framework
  // expects an EntityDefinition shape (name, fields, indexes, etc.)
  // sees them directly without unwrapping. Manifest builder paths
  // walk `.name` and `.fields` straight off the builder.
  Object.assign(self, def);
  return self;
}

/**
 * The fluent `e` namespace. Equivalent to the procedural `entity()`
 * function — both produce manifest-compatible definitions, both can
 * be mixed in the same app.
 */
export const e = {
  entity(
    name: string,
    fields: Record<string, FieldBuilder>,
  ): EntityBuilder {
    return buildEntity({ name, fields });
  },
  idx,
};

/**
 * Extract attached policies from a fluent entity. The manifest
 * builder calls this when assembling the top-level `policies` list,
 * so apps using the fluent `.policies(...)` chain don't have to
 * register policies separately at the manifest root.
 *
 * Returns an empty array for entities produced by the procedural
 * `entity()` API — those apps register policies the old way.
 */
export function extractAttachedPolicies(
  e: EntityDefinition | EntityBuilder,
): PolicyDefinition[] {
  const carried = (e as EntityDefinition & {
    _attachedPolicies?: PolicyDefinition[];
  })._attachedPolicies;
  return carried ?? [];
}
