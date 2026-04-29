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
  /** Per-app trusted origins for OAuth `?callback=` validation. Merged with `PYLON_TRUSTED_ORIGINS` env. */
  trustedOrigins?: string[];
};

export type ManifestAuthConfig = {
  user: {
    entity: string;
    expose: string[];
    hide: string[];
  };
  session: {
    expires_in: number;
    cookie_cache: {
      enabled: boolean;
      max_age: number;
      claims: string[];
    };
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
    },
    session: {
      expires_in: cfg.session?.expiresIn ?? 30 * 24 * 60 * 60,
      cookie_cache: {
        enabled: cfg.session?.cookieCache?.enabled ?? false,
        max_age: cfg.session?.cookieCache?.maxAge ?? 5 * 60,
        claims: cfg.session?.cookieCache?.claims ?? ["is_admin", "tenant_id"],
      },
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
