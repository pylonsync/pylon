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

/**
 * Insert-time default marker stored on a field definition. Apps never
 * construct these directly — they come from `.default(v)` / `.defaultNow()`
 * / `.owner()` on a field builder. The runtime + codegen read them.
 */
export type DefaultMarker =
  | { kind: "value"; value: unknown }
  | { kind: "now" }
  | { kind: "owner" };

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
   * When true, the field is stripped from every REPLICATION surface —
   * snapshot pull, delta pull, WS change fanout, `sync=1` cursor
   * fetches — but stays on direct reads (entity get/list, queries,
   * SSR serverData) and inside server functions.
   *
   * This is the "heavy but not secret" modifier: a large JSON/blob
   * column (render plans, slide decks, generated markdown) should not
   * ride into every browser's replica on every sync, but the detail
   * view that needs it can still fetch it by id. Contrast
   * `serverOnly`, which hides a field from ALL client surfaces.
   */
  syncOmit?: boolean;
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
  /**
   * When true, the field is AEAD-encrypted at rest. The framework
   * encrypts the value before writing to SQLite/Postgres and
   * decrypts on read. Cipher: ChaCha20-Poly1305. Key:
   * `PYLON_ENCRYPTION_KEY` env (32 bytes, hex or base64).
   *
   * Use for PII / secrets that must survive a DB-file leak: API
   * keys stored on rows, social security numbers, OAuth tokens.
   * Plaintext only exists inside the Pylon process.
   *
   * Restrictions:
   * - Encrypted fields are NOT queryable. `ctx.db.lookup` /
   *   `WHERE encryptedField = 'x'` always returns nothing because
   *   each write produces fresh ciphertext.
   * - Cannot combine with `unique: true`.
   * - Valid only on `string`, `richtext`, and JSON-shaped fields.
   *
   * Decryption pre-pass means rows written BEFORE the field was
   * annotated `encrypted: true` continue to read fine (passes
   * through as plaintext). Next write through the mutation
   * pipeline upgrades them to ciphertext.
   */
  encrypted?: boolean;
  /**
   * Insert-time default. Set via `.default(value)` / `.defaultNow()` /
   * `.owner()` on the field builder; recorded for the runtime + codegen.
   */
  default?: DefaultMarker;
  /**
   * Allowed values for `field.enum([...])` — recorded so codegen emits a
   * precise literal-union type instead of a wide `string`.
   */
  enumValues?: readonly string[];
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
   * Keep the field out of client REPLICAS while leaving direct reads
   * intact. See [`FieldDefinition.syncOmit`] for the full semantics.
   *
   * Example: `slidesJson: field.string().syncOmit()` stops a multi-KB
   * render plan from streaming into every browser on every sync; the
   * editor still gets it via `db.get("Variant", id)`.
   */
  syncOmit(): FieldBuilder;
  /**
   * Mark the field as set-on-insert-only. See [`FieldDefinition.readonly`]
   * for the full semantics.
   *
   * Example: `authorId: field.id("User").readonly()` lets the framework
   * reject any `PATCH` payload trying to rewrite the row's author —
   * closes the IDOR-via-update-payload class.
   */
  readonly(): FieldBuilder;
  /**
   * Mark the field as AEAD-encrypted at rest. See
   * [`FieldDefinition.encrypted`] for the full semantics +
   * restrictions.
   *
   * Example: `apiKey: field.string().serverOnly().encrypted()`
   * keeps the key out of HTTP responses AND encrypts the bytes
   * sitting in SQLite.
   */
  encrypted(): FieldBuilder;
  /**
   * Mark the field as the row's **owner** — the framework stamps it
   * with `auth.userId` on every insert and refuses any client attempt
   * to set it to a different user. This is what makes *optimistic,
   * local-first writes the default* for owned data: the client can
   * `db.insert("Offer", { buyerId, … })` directly (paints the row in
   * the local store instantly, no server round-trip) and still be
   * secure — the server overwrites/validates the owner from the
   * session, so a forged `buyerId` is rejected before it ever lands.
   *
   * Concretely `.owner()`:
   * - on **insert**, fills the field from `auth.userId` when omitted,
   *   and rejects a non-admin caller who supplies a *different*
   *   non-empty value (`OWNER_MISMATCH`, 403) — closing the IDOR
   *   shape where a policy gates on `data.ownerId == auth.userId`
   *   but the attacker just sends someone else's id;
   * - on **update**, behaves like [`readonly`] — the owner can't be
   *   reassigned through the HTTP entity routes.
   *
   * Reach for this instead of writing a server function whenever the
   * only server-authoritative part of a create is "who made it":
   * `authorId`, `buyerId`, `sellerId`, `createdBy`, `userId`. Guests
   * count — their stable guest id is stamped. Admin contexts may set
   * an explicit value (migrations, tooling).
   *
   * Example: `sellerId: field.string().owner()` lets a listing be
   * created with a plain optimistic `db.insert` while the seller id
   * stays unspoofable.
   */
  owner(): FieldBuilder;
  /**
   * Set a static insert-time default, recorded for the runtime +
   * codegen. Example: `enabled: field.bool().default(true)`.
   */
  default(value: unknown): FieldBuilder;
  /**
   * Default a datetime field to insert-time `now()`. Example:
   * `createdAt: field.datetime().defaultNow()`.
   */
  defaultNow(): FieldBuilder;
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
    syncOmit() {
      return buildField({ ...def, syncOmit: true });
    },
    readonly() {
      return buildField({ ...def, readonly: true });
    },
    encrypted() {
      return buildField({ ...def, encrypted: true });
    },
    owner() {
      // Dynamic default filled by the auth-aware pipeline; also lock
      // the field on update so ownership can't be reassigned via PATCH.
      return buildField({ ...def, readonly: true, default: { kind: "owner" } });
    },
    default(value: unknown) {
      return buildField({ ...def, default: { kind: "value", value } });
    },
    defaultNow() {
      return buildField({ ...def, default: { kind: "now" } });
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
  /**
   * `field.enum(["pending", "paid", "failed"])` — stored as a string with
   * allowed-values metadata so codegen emits a precise literal union
   * (`"pending" | "paid" | "failed"`) instead of a wide `string`.
   */
  enum: (values: readonly string[]) =>
    buildField({
      type: "string",
      optional: false,
      unique: false,
      enumValues: values,
    }),
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
  /**
   * Client replication. Default `true` — the entity is bulk-snapshotted +
   * delta-streamed into every client's local replica. Set `false` for a large,
   * server-queried catalog (e.g. a 10k-row product table) the client reaches via
   * `db.useSearch` + by-id fetch instead of holding the whole table locally:
   * `sync: false` keeps it out of the replica entirely. Direct reads
   * (`/api/entities/X`, `/api/search/X`) and policies are unchanged.
   *
   * Pass an object to keep the entity live but bound WHICH rows replicate —
   * see {@link SyncScope}. That's the middle setting between "every row you
   * can read" and "nothing".
   */
  sync?: boolean | SyncScope;
}

/**
 * Bounded replication for an entity that should stay live but shouldn't ship
 * its whole history to every client.
 *
 * ```ts
 * entity("Message", { … }, {
 *   sync: { where: "data.roomId == auth.tenantId", limit: 5_000 },
 * })
 * ```
 *
 * `where` is the SAME expression language as policies (including
 * `exists(...)`), evaluated per row per caller. It is applied ON TOP of the
 * read policy, never instead of it — a scope can only remove rows from a
 * replica, so getting one wrong is a missing-data bug, never a leak.
 *
 * Applies to REPLICATION only: the snapshot, the sync engine's bootstrap, and
 * the change-log delta. Direct reads (`/api/entities/X`, `/api/search/X`) are
 * unchanged, exactly as with `sync: false`.
 *
 * A row that leaves scope is pushed to clients as a DELETE, so replicas evict
 * it rather than holding a copy that never updates again.
 */
export interface SyncScope {
  /** Policy-DSL predicate. Omit to bound by `limit` alone. */
  where?: string;
  /**
   * Hard ceiling on rows replicated per client for this entity. Belt to
   * `where`'s braces — a predicate that turns out not to bound growth still
   * can't flood a replica. Survives snapshot pagination.
   */
  limit?: number;
}

export function entity(
  name: string,
  fields: Record<string, FieldBuilder>,
  options?: {
    indexes?: IndexDefinition[];
    relations?: RelationDefinition[];
    search?: SearchConfig;
    /** `false` to keep out of the replica, or a {@link SyncScope} to bound
     *  which rows reach it. Mirrors {@link EntityDefinition.sync} — this is
     *  the type callers actually hit, so the two must not drift. */
    sync?: boolean | SyncScope;
  },
): EntityDefinition {
  return {
    name,
    fields,
    indexes: options?.indexes,
    relations: options?.relations,
    search: options?.search,
    sync: options?.sync,
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
   * Layout module path chain (root→leaf). Each layout wraps the next
   * as `children`. Only relevant for `mode === "ssr"`.
   */
  layouts?: string[];
  /**
   * Route kind. Omitted (or `"page"`) is a normal navigable page.
   * `"not-found"` / `"error"` are SSR boundary modules discovered from
   * `app/.../not-found.tsx` and `app/.../error.tsx`. Boundary routes are
   * NOT matched as navigable URLs — the host renders `not-found` for
   * unmatched URLs (HTTP 404) and `error` on render failure (HTTP 500).
   * `path` records the segment prefix the boundary covers (`/` for root).
   * `"route"` is a form/method handler (`app/.../route.ts` exporting
   * POST/PUT/PATCH/DELETE) — matched on its `path` for non-GET requests only,
   * never rendered as a page.
   */
  kind?:
    | "page"
    | "not-found"
    | "error"
    | "route"
    | "sitemap"
    | "robots"
    | "og-image";
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

/**
 * Serialized form of `field.X().owner()`. Carried in a field's
 * `default` slot as a *dynamic* default — the value isn't known until
 * a request arrives, so the framework's auth-aware mutation pipeline
 * (OwnerStampPlugin, Rust side) fills it from `auth.userId` on insert
 * and rejects any client attempt to set it to a different user. The
 * storage-layer default-filler (`apply_field_defaults`) deliberately
 * skips this shape — it has no auth context, so it can't (and must
 * not) stamp an owner.
 *
 * `$auth: "userId"` is the only kind today; the object shape leaves
 * room for future auth-derived defaults (e.g. `{$auth:"tenantId"}`)
 * without another manifest migration.
 */
export interface OwnerDefaultSentinel {
  $auth: "userId";
}

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
  /** Set when the field is `field.X().syncOmit()` — see
   *  [`FieldDefinition.syncOmit`]. Omitted by default. */
  syncOmit?: boolean;
  /** Set when the field is `field.X().readonly()` — see
   *  [`FieldDefinition.readonly`]. Omitted by default. */
  readonly?: boolean;
  /** Default value to fill on insert when the row omits this field.
   *  - `"now"`     → runtime stamps the current UTC time
   *  - `{$auth:…}` → a *dynamic* default the auth-aware mutation
   *                  pipeline fills (see [`OwnerDefaultSentinel`]).
   *                  Never stamped at the storage layer — the
   *                  OwnerStampPlugin fills + enforces it.
   *  - any literal → runtime stamps that exact value
   *  Maps to `field.X().defaultNow()` / `.default(value)` /
   *  `field.X().owner()`. */
  default?: "now" | string | number | boolean | null | OwnerDefaultSentinel;
  /** Allowed values for `field.enum([...])` — recorded so codegen
   *  can emit a literal-union type and runtime validation can
   *  reject out-of-set inserts. Plain `field.string()` doesn't
   *  carry this; only `field.enum()`. */
  enumValues?: readonly string[];
  /** Set when the field is `field.X().encrypted()` — AEAD-encrypted
   *  at rest. See [`FieldDefinition.encrypted`]. */
  encrypted?: boolean;
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
  /** Client replication; omitted when true (the default). `false` keeps the
   *  entity out of the client replica (snapshot + delta) — server-queried only. */
  sync?: boolean;
  /**
   * Replication scope, flattened out of {@link SyncScope}. Sibling keys
   * rather than a nested object so `sync` keeps its boolean shape on the
   * wire — an older binary reading this manifest still sees `sync: true`
   * and replicates everything, which is the right way for a version skew
   * to degrade.
   */
  sync_scope?: string;
  sync_limit?: number;
}

export interface ManifestRoute {
  path: string;
  mode: string;
  query?: string;
  auth?: string;
  component?: string;
  layouts?: string[];
  /** "not-found" / "error" boundaries, or "route" form handlers; omitted for normal pages. */
  kind?:
    | "page"
    | "not-found"
    | "error"
    | "route"
    | "sitemap"
    | "robots"
    | "og-image";
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

/** A recurring job: run a function on a cron schedule. Declared in the
 *  manifest via `cron(schedule, functionName)`; the runtime fires the named
 *  function with anonymous auth every time the schedule matches. Server-side
 *  `ctx.db.*` is trusted, so a maintenance handler reads/writes its entities
 *  directly without elevating. */
export interface ManifestCron {
  /** Standard 5-field cron expression, e.g. `"0 * * * *"` (every hour). */
  schedule: string;
  /** Name of the function (query/mutation/action) to run. Should be an
   *  `internal: true` function so it isn't also reachable over HTTP. */
  function: string;
  /** Optional human description, surfaced by `pylon status` / tooling. */
  description?: string;
}

/** A self-hosted web font. Declared via `font({...})`; at build the runtime
 *  fetches the family from Google Fonts, self-hosts the woff2 same-origin,
 *  generates `@font-face` + a size-adjusted fallback face (zero layout shift),
 *  and auto-injects the preload + faces into every SSR page's `<head>`. The app
 *  references the font through the CSS `variable`. */
export interface ManifestFont {
  /** Google Fonts family name, e.g. "Geist" or "Inter". */
  family: string;
  /** CSS custom property the app uses to apply the font, e.g. "--font-geist".
   *  Set `font-family: var(--font-geist)` (it resolves to the family + the
   *  size-adjusted fallback + your `fallback` stack). */
  variable: string;
  /** Weights to load — specific (`["400","500","700"]`) or a CSS2 range
   *  (`["300..700"]`). Defaults to `["400"]`. */
  weights?: string[];
  /** Styles to load. Defaults to `["normal"]`. */
  styles?: ("normal" | "italic")[];
  /** Unicode subsets, e.g. `["latin"]`. Defaults to `["latin"]`. */
  subsets?: string[];
  /** CSS `font-display`. Defaults to `"swap"`. */
  display?: "auto" | "block" | "swap" | "fallback" | "optional";
  /** Emit `<link rel="preload">` for the font files so the browser fetches
   *  them early. Defaults to `true`. */
  preload?: boolean;
  /** Fallback stack appended after the family + the adjusted fallback face,
   *  e.g. `["system-ui", "sans-serif"]`. Defaults to `["sans-serif"]`. */
  fallback?: string[];
  /** Generate a size-adjusted fallback `@font-face` (matching x-height +
   *  metrics) so there's no layout shift when the web font swaps in. Defaults
   *  to `true`. Serialized snake_case to match the kernel manifest wire shape. */
  adjust_font_fallback?: boolean;
}

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
  /** App-level LLM provider config. Optional — env wins when set. */
  llm?: ManifestLlmConfig;
  /** Declared OAuth integrations. Auto-creates the `_Connection`
   *  entity at runtime boot. */
  connections?: ManifestConnection[];
  /** Recurring jobs — run a function on a cron schedule. */
  crons?: ManifestCron[];
  /** Self-hosted web fonts. Fetched + self-hosted at build; preload +
   *  `@font-face` auto-injected into the SSR `<head>`. */
  fonts?: ManifestFont[];
}

/**
 * Declare a recurring job. Runs the named function every time the cron
 * `schedule` matches (the runtime checks once a minute).
 *
 * ```ts
 * buildManifest({
 *   // ...
 *   crons: [
 *     cron("0 * * * *", "hourlyRollup"),     // every hour, on the hour
 *     cron("15 3 * * *", "nightlyCleanup"),   // daily at 03:15
 *   ],
 * });
 * ```
 *
 * `functionName` is a function in `functions/` — make it `internal: true`
 * so it isn't also exposed over HTTP. It runs with anonymous auth, and a
 * function's own `ctx.db.*` calls are server-side (not policy-gated), so a
 * maintenance handler writes directly. Only call
 * `ctx.auth.elevate({ admin: true, reason: "..." })` if it chains an
 * `internal: true` function via `ctx.scheduler`, or you run with
 * `PYLON_STRICT_FN_POLICIES=1`. The `reason` is mandatory (audited).
 *
 * Multiple replicas: each Pylon process runs its own scheduler. On a SHARED
 * datastore (Postgres — the cloud default), the runtime takes a per-minute
 * lease so each cron fires exactly ONCE per tick across all replicas. On
 * per-replica SQLite there's no shared lease, so each replica fires — keep
 * handlers idempotent there (most maintenance work naturally is). A
 * single-machine app always fires exactly once.
 */
export function cron(
  schedule: string,
  functionName: string,
  opts?: { description?: string }
): ManifestCron {
  return {
    schedule,
    function: functionName,
    ...(opts?.description ? { description: opts.description } : {}),
  };
}

/**
 * Declare a self-hosted web font (Google Fonts). At build, Pylon fetches the
 * family, self-hosts the woff2 same-origin, generates `@font-face` + a
 * size-adjusted fallback face, and auto-injects the preload + faces into every
 * SSR page's `<head>` — no render-blocking third-party request, no layout shift.
 *
 * ```ts
 * buildManifest({
 *   // ...
 *   fonts: [
 *     font({ family: "Geist", variable: "--font-sans", weights: ["300..700"], subsets: ["latin"] }),
 *     font({ family: "Geist Mono", variable: "--font-mono", weights: ["400..600"] }),
 *   ],
 * });
 * ```
 *
 * Then use it in CSS: `font-family: var(--font-sans);` (resolves to the family,
 * the zero-CLS fallback, then your `fallback` stack).
 */
export function font(opts: {
  family: string;
  variable: string;
  weights?: string[];
  styles?: ("normal" | "italic")[];
  subsets?: string[];
  display?: "auto" | "block" | "swap" | "fallback" | "optional";
  preload?: boolean;
  fallback?: string[];
  adjustFontFallback?: boolean;
}): ManifestFont {
  return {
    family: opts.family,
    variable: opts.variable,
    weights: opts.weights ?? ["400"],
    styles: opts.styles ?? ["normal"],
    subsets: opts.subsets ?? ["latin"],
    display: opts.display ?? "swap",
    preload: opts.preload ?? true,
    ...(opts.fallback ? { fallback: opts.fallback } : {}),
    adjust_font_fallback: opts.adjustFontFallback ?? true,
  };
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
        if (fb._def.syncOmit) {
          f.syncOmit = true;
        }
        if (fb._def.readonly) {
          f.readonly = true;
        }
        if (fb._def.encrypted) {
          f.encrypted = true;
        }
        // `default` + `enumValues` are surfaced on the fluent
        // FieldBuilder via the v0.4 SDK. Read off the private
        // backing slot so both APIs serialize identically — apps
        // using the procedural `field` exports get the same
        // ManifestField shape as fluent apps.
        const extra = fb._def;
        if (extra.default) {
          f.default =
            extra.default.kind === "now"
              ? "now"
              : extra.default.kind === "owner"
                ? // Dynamic, auth-derived default. Carried as a sentinel
                  // object the Rust mutation pipeline recognizes; the
                  // storage-layer default-filler skips it (no auth there).
                  ({ $auth: "userId" } satisfies OwnerDefaultSentinel)
                : (extra.default.value as ManifestField["default"]);
        }
        if (extra.enumValues && extra.enumValues.length > 0) {
          f.enumValues = extra.enumValues;
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
    // Emit only when opted OUT — the runtime defaults sync to true.
    if (e.sync === false) {
      result.sync = false;
    } else if (e.sync && typeof e.sync === "object") {
      // A SCOPE. Flattened into sibling keys rather than nested under `sync`
      // so the field keeps its boolean shape on the wire: an older binary
      // reading this manifest still sees `sync: true` and replicates
      // everything — more data than intended, which is the right way for
      // this to degrade. Nesting would make `sync` fail to deserialize as a
      // bool and take the whole app down on a version skew.
      const scope = e.sync;
      if (scope.where !== undefined) result.sync_scope = scope.where;
      if (scope.limit !== undefined) result.sync_limit = scope.limit;
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
    if (r.kind && r.kind !== "page") result.kind = r.kind;
    return result;
  });
}

/**
 * Walk the project's `app/` directory and discover file-based SSR
 * routes. Returns `RouteDefinition[]` ready to slot into
 * `buildManifest({ routes })`.
 *
 * Mapping (Next App Router-shaped):
 *   - `app/page.tsx` → `/`
 *   - `app/about/page.tsx` → `/about`
 *   - `app/blog/[slug]/page.tsx` → `/blog/:slug`
 *   - `app/layout.tsx` wraps every page below
 *   - `app/(marketing)/about/page.tsx` → `/about` (group strip)
 *
 * Sorts deterministically — literal segments before parameterized
 * ones at each depth — so the Rust matcher's first-match-wins
 * lookup picks the right route.
 *
 * `not-found.tsx` / `error.tsx` boundaries are emitted as `kind`-tagged
 * routes here; `loading.tsx` is resolved at render time by the SSR runtime
 * (filesystem walk), so it needs no discovery entry.
 */
export async function discoverAppRoutes(opts?: {
  appDir?: string;
}): Promise<RouteDefinition[]> {
  // Pull node fs/path lazily. @pylonsync/sdk has no @types/node dep
  // (kept light so client bundles stay tiny), so we type as `any`
  // and validate at runtime — users call this from app.ts under
  // Bun/Node, where the requires resolve.
  //
  // ESM Bun/Node doesn't expose `require` on globalThis. Build one
  // via createRequire(import.meta.url) — the standard ESM →
  // node-builtins escape hatch. The optional-chain on `require`
  // covers the browser case (returns undefined, we fall through to
  // []).
  let fs: any;
  let path: any;
  try {
    const nodeReq = (globalThis as any).require ??
      (await import("node:module")).createRequire(import.meta.url);
    fs = nodeReq("node:fs");
    path = nodeReq("node:path");
  } catch {
    // Browser bundle hit — there's no `app/` to discover. Returning
    // [] keeps the function safe to import from client code too.
    return [];
  }
  if (!fs || !path) return [];

  const cwd = (globalThis as any).process?.cwd?.() ?? ".";
  const appDir =
    opts?.appDir && path.isAbsolute(opts.appDir)
      ? opts.appDir
      : path.join(cwd, opts?.appDir ?? "app");
  if (!fs.existsSync(appDir) || !fs.statSync(appDir).isDirectory()) {
    return [];
  }

  type PageHit = {
    segments: string[];
    component: string;
    layouts: string[];
  };
  type BoundaryHit = {
    segments: string[];
    component: string;
    layouts: string[];
    kind: "not-found" | "error";
  };
  type RouteHandlerHit = { segments: string[]; component: string };
  const pages: PageHit[] = [];
  const boundaries: BoundaryHit[] = [];
  const routeHandlers: RouteHandlerHit[] = [];
  // `opengraph-image.{tsx,ts,jsx,js}` colocated with any segment. Each
  // default-exports a function returning an `ImageResponse` (or React
  // element); the SSR runtime renders it to a PNG on demand and pages under
  // the segment get an auto `og:image` meta tag pointing at it.
  const ogImages: RouteHandlerHit[] = [];

  // Resolve the first existing `<base>.{tsx,ts,jsx,js}` in `dir` and
  // return it as a cwd-relative, extension-less module path (or null).
  const findModule = (dir: string, base: string): string | null => {
    const hit = [`${base}.tsx`, `${base}.ts`, `${base}.jsx`, `${base}.js`]
      .map((n: string) => path.join(dir, n))
      .find((p: string) => fs.existsSync(p));
    return hit ? path.relative(cwd, hit).replace(/\.(tsx?|jsx?)$/, "") : null;
  };

  function walk(dir: string, segments: string[], layouts: string[]): void {
    let entries: Array<{ name: string; isDirectory(): boolean }>;
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    const layoutHere = findModule(dir, "layout");
    const nextLayouts = layoutHere ? [...layouts, layoutHere] : layouts;
    const pageHere = findModule(dir, "page");
    if (pageHere) {
      pages.push({
        segments: [...segments],
        component: pageHere,
        layouts: nextLayouts,
      });
    }
    // Boundary modules (not-found.tsx / error.tsx) co-located with a
    // segment. They wrap in the layouts ABOVE them (nextLayouts) so the
    // root not-found renders inside the root shell. The host consults
    // these for unmatched-URL 404s + render-failure 500s.
    const notFoundHere = findModule(dir, "not-found");
    if (notFoundHere) {
      boundaries.push({
        segments: [...segments],
        component: notFoundHere,
        layouts: nextLayouts,
        kind: "not-found",
      });
    }
    const errorHere = findModule(dir, "error");
    if (errorHere) {
      boundaries.push({
        segments: [...segments],
        component: errorHere,
        layouts: nextLayouts,
        kind: "error",
      });
    }
    // Form/method handler (`route.ts` exporting POST/PUT/PATCH/DELETE). Matched
    // on its path for non-GET requests only — never rendered, no layouts.
    const routeHere = findModule(dir, "route");
    if (routeHere) {
      routeHandlers.push({ segments: [...segments], component: routeHere });
    }
    // Dynamic OG image colocated with this segment. Rendered to PNG on
    // demand; never navigable/hydrated.
    const ogImageHere = findModule(dir, "opengraph-image");
    if (ogImageHere) {
      ogImages.push({ segments: [...segments], component: ogImageHere });
    }
    for (const e of entries) {
      if (!e.isDirectory()) continue;
      if (e.name.startsWith(".") || e.name === "node_modules") continue;
      const sub = path.join(dir, e.name);
      const isGroup = e.name.startsWith("(") && e.name.endsWith(")");
      const newSegments = isGroup ? segments : [...segments, e.name];
      walk(sub, newSegments, nextLayouts);
    }
  }
  walk(appDir, [], []);

  // Segment kinds, least-to-most greedy:
  //   static            "blog"          rank 0  (most specific)
  //   dynamic param     "[slug]"        rank 1  → :slug
  //   catch-all         "[...slug]"     rank 2  → *slug   (matches ≥1 segment)
  //   optional catch-all"[[...slug]]"   rank 3  → *?slug  (matches ≥0 segments)
  // A catch-all is only valid as the LAST segment of a route (it consumes
  // the rest of the path); Next.js enforces the same.
  const isOptionalCatchAll = (s: string): boolean =>
    s.startsWith("[[...") && s.endsWith("]]");
  const isCatchAll = (s: string): boolean =>
    s.startsWith("[...") && s.endsWith("]") && !isOptionalCatchAll(s);
  const isParam = (s: string): boolean =>
    s.startsWith("[") && s.endsWith("]") && !isCatchAll(s) && !isOptionalCatchAll(s);
  const segRank = (s: string): number =>
    isOptionalCatchAll(s) ? 3 : isCatchAll(s) ? 2 : isParam(s) ? 1 : 0;
  pages.sort((a, b) => {
    const minLen = Math.min(a.segments.length, b.segments.length);
    for (let i = 0; i < minLen; i++) {
      const ar = segRank(a.segments[i]);
      const br = segRank(b.segments[i]);
      if (ar !== br) return ar - br; // more specific (lower rank) first
      if (a.segments[i] !== b.segments[i]) {
        return a.segments[i] < b.segments[i] ? -1 : 1;
      }
    }
    return a.segments.length - b.segments.length;
  });

  // Convert a discovered file segment to its route-pattern token. The Rust
  // matcher (frontend.rs `match_ssr_route`) reads these markers back:
  //   :name   dynamic param      *name   catch-all      *?name  optional catch-all
  const segmentToToken = (s: string): string => {
    if (isOptionalCatchAll(s)) return `*?${s.slice(5, -2)}`; // [[...x]] → *?x
    if (isCatchAll(s)) return `*${s.slice(4, -1)}`; //          [...x]   → *x
    if (isParam(s)) return `:${s.slice(1, -1)}`; //             [x]      → :x
    return s;
  };
  const segmentsToPath = (segments: string[]): string =>
    "/" + segments.map(segmentToToken).join("/");

  const pageRoutes: RouteDefinition[] = pages.map((p) => ({
    path: segmentsToPath(p.segments),
    mode: "ssr" as const,
    component: p.component,
    layouts: p.layouts,
  }));

  // Boundary routes carry `kind` and the segment-prefix path. The host
  // never matches them as navigable URLs; it picks the longest-prefix
  // not-found for an unmatched URL. Sorted deepest-first so prefix
  // selection is stable.
  const boundaryRoutes: RouteDefinition[] = boundaries
    .sort((a, b) => b.segments.length - a.segments.length)
    .map((b) => ({
      path: segmentsToPath(b.segments),
      mode: "ssr" as const,
      component: b.component,
      layouts: b.layouts,
      kind: b.kind,
    }));

  // Form/method handlers. Keyed by the SAME path token form as pages
  // (`:param` / `*catch-all`), matched by the host for non-GET methods only.
  const routeRoutes: RouteDefinition[] = routeHandlers.map((r) => ({
    path: segmentsToPath(r.segments),
    mode: "ssr" as const,
    component: r.component,
    kind: "route" as const,
  }));

  // Root-level data conventions (Next-style): `app/sitemap.{ts,tsx,js,jsx}` →
  // `/sitemap.xml`, `app/robots.{...}` → `/robots.txt`. Each exports a default
  // (optionally async) function returning sitemap entries / a robots object;
  // the SSR runtime detects these by component basename, calls the export, and
  // serializes the return to XML / plain text (no React render). Root-level
  // only for now (no route-group nesting / `generateSitemaps` id sharding).
  const dataRoutes: RouteDefinition[] = [];
  const sitemapMod = findModule(appDir, "sitemap");
  if (sitemapMod) {
    dataRoutes.push({
      path: "/sitemap.xml",
      mode: "ssr",
      component: sitemapMod,
      kind: "sitemap",
    });
  }
  const robotsMod = findModule(appDir, "robots");
  if (robotsMod) {
    dataRoutes.push({
      path: "/robots.txt",
      mode: "ssr",
      component: robotsMod,
      kind: "robots",
    });
  }

  // Dynamic OG image routes: `<segment>/opengraph-image` (root → just
  // `/opengraph-image`). Matched like a page (params flow through) but
  // served as a PNG by the SSR runtime — never rendered as HTML or hydrated.
  const ogImageRoutes: RouteDefinition[] = ogImages.map((o) => ({
    path:
      o.segments.length === 0
        ? "/opengraph-image"
        : `${segmentsToPath(o.segments)}/opengraph-image`,
    mode: "ssr" as const,
    component: o.component,
    kind: "og-image" as const,
  }));

  return [
    ...pageRoutes,
    ...boundaryRoutes,
    ...routeRoutes,
    ...dataRoutes,
    ...ogImageRoutes,
  ];
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

// ---------------------------------------------------------------------------
// LLM provider configuration
// ---------------------------------------------------------------------------

/**
 * Developer-facing camelCase config consumed by the `llm({...})`
 * factory. All fields optional; environment variables
 * (`PYLON_LLM_PROVIDER`, `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
 * `PYLON_LLM_MODEL`) take precedence so operators can override per
 * deploy without redeploying the bundle.
 */
export type LlmConfig = {
  /** Provider name. Default: env detection. */
  provider?: "anthropic" | "openai";
  /** Default model when the caller doesn't pass `model`. */
  defaultModel?: string;
  /**
   * Allowlist of models callers may request via the `model` field.
   * Empty = no extra allowance beyond what `PYLON_AI_MODELS_ALLOWED`
   * env provides. Non-admin callers can't request models outside
   * this list.
   */
  allowedModels?: string[];
};

export type ManifestLlmConfig = {
  provider?: "anthropic" | "openai";
  default_model?: string;
  allowed_models?: string[];
};

// ---------------------------------------------------------------------------
// Connection (per-user OAuth integrations)
// ---------------------------------------------------------------------------

/**
 * Developer-facing config for `defineConnection({...})`. Each entry
 * adds a `ctx.connections.<name>` surface to mutation + action ctx
 * (server-side OAuth tokens, never visible to the browser).
 *
 * `provider` selects the OAuth client wire shape from pylon-auth's
 * built-in list (`google`, `github`, `slack`, `microsoft`, etc.).
 * `name` is the app-facing key — different connections can target
 * the same provider with different scopes
 * (e.g. `google-calendar` vs `google-drive`).
 *
 * Configuration: per-provider client id + secret come from env
 * (`PYLON_OAUTH_<PROVIDER>_CLIENT_ID`, `PYLON_OAUTH_<PROVIDER>_CLIENT_SECRET`).
 * Callback URL is derived from `PYLON_PUBLIC_URL` +
 * `/api/connections/<name>/callback`.
 *
 * Storage: the framework auto-creates a `_Connection` entity at
 * boot when any connection is declared; token fields are AEAD-
 * encrypted at rest (`PYLON_ENCRYPTION_KEY` is REQUIRED — boot
 * fails without it when connections are declared).
 */
export type ConnectionConfig = {
  /** App-facing key. `ctx.connections.get(name)` matches on this. */
  name: string;
  /** Provider identifier matching pylon-auth's OAuth client. */
  provider: string;
  /** Whitespace-separated scopes. Empty = provider default. */
  scopes?: string;
};

export type ManifestConnection = {
  name: string;
  provider: string;
  scopes?: string;
};

/**
 * Declare a server-side OAuth integration. Returns the manifest
 * entry the runtime parses. Re-exported through `app.ts`:
 *
 * ```ts
 * import { defineConnection } from "@pylonsync/sdk";
 *
 * export const googleConn = defineConnection({
 *   name: "google",
 *   provider: "google",
 *   scopes: "email profile https://www.googleapis.com/auth/calendar.readonly",
 * });
 * ```
 *
 * `buildManifest({ connections: [googleConn] })` carries this into
 * the manifest; the runtime auto-creates the `_Connection` entity
 * and exposes `ctx.connections.get("google")` to actions.
 */
export function defineConnection(cfg: ConnectionConfig): ManifestConnection {
  return {
    name: cfg.name,
    provider: cfg.provider,
    ...(cfg.scopes ? { scopes: cfg.scopes } : {}),
  };
}

/**
 * Build the manifest's `llm` block from the user-facing camelCase
 * config. Returns the snake_case shape the Rust runtime parses.
 *
 * ```ts
 * export default {
 *   llm: llm({
 *     provider: "anthropic",
 *     defaultModel: "claude-sonnet-4-5",
 *     allowedModels: ["claude-sonnet-4-5", "claude-haiku-4-5"],
 *   }),
 * }
 * ```
 */
export function llm(cfg: LlmConfig = {}): ManifestLlmConfig {
  const out: ManifestLlmConfig = {};
  if (cfg.provider) out.provider = cfg.provider;
  if (cfg.defaultModel) out.default_model = cfg.defaultModel;
  if (cfg.allowedModels && cfg.allowedModels.length > 0) {
    out.allowed_models = cfg.allowedModels;
  }
  return out;
}

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
  llm?: ManifestLlmConfig;
  connections?: ManifestConnection[];
  crons?: ManifestCron[];
  fonts?: ManifestFont[];
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
    ...(options.llm && Object.keys(options.llm).length > 0
      ? { llm: options.llm }
      : {}),
    ...(options.connections && options.connections.length > 0
      ? { connections: options.connections }
      : {}),
    ...(options.crons && options.crons.length > 0
      ? { crons: options.crons }
      : {}),
    ...(options.fonts && options.fonts.length > 0
      ? { fonts: options.fonts }
      : {}),
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

// `field` builders (`default` / `defaultNow` / `owner` / `enum`) are all
// implemented directly on `buildField` above and typed on the FieldBuilder
// interface, so `field.datetime().defaultNow()` and `field.enum([...])`
// are real, statically-typed chainables — no runtime augmentation needed.

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
