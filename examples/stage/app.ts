import { entity, field, policy, buildManifest } from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// Stage — a block-based multi-page site builder.
//
// Sites own Pages own Blocks. Every block edit is a mutation that
// broadcasts a change event, so two editors open on the same page see
// each other's work land in real time — no app-specific socket.
// ---------------------------------------------------------------------------

const User = entity("User", {
  email: field.string().unique(),
  displayName: field.string(),
  avatarColor: field.string(),
  createdAt: field.datetime(),
});

const Organization = entity("Organization", {
  name: field.string(),
  slug: field.string().unique(),
  createdBy: field.id("User"),
  createdAt: field.datetime(),
});

const OrgMember = entity(
  "OrgMember",
  {
    userId: field.id("User"),
    orgId: field.id("Organization"),
    role: field.string(),
    joinedAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_org_user", fields: ["orgId", "userId"], unique: true },
      { name: "by_user", fields: ["userId"], unique: false },
    ],
  },
);

// A Site is what users publish. `slug` is globally unique so the public
// preview route /p/:slug can look it up without needing a tenant. Published
// sites render live content; draft edits are gated to workspace members.
//
// `tokensJson` is the site's design system — color + type + radius tokens
// the user can edit live. Matches the DESIGN.md shape (minus components)
// so a site's tokens can be exported as a DESIGN.md file for downstream
// coding agents. Nullable for forward-compat: old sites without tokens
// fall back to the bundled defaults in the client renderer.
const Site = entity(
  "Site",
  {
    orgId: field.id("Organization"),
    name: field.string(),
    slug: field.string().unique(), // public URL component
    faviconEmoji: field.string(),
    accentColor: field.string(), // hex e.g. "#ec4899" (legacy; see tokensJson)
    typeface: field.string(), // "sans" | "serif" | "mono"
    tokensJson: field.string().optional(), // per-site design tokens (DESIGN.md-ish)
    createdBy: field.id("User"),
    createdAt: field.datetime(),
    publishedAt: field.datetime().optional(),
  },
  {
    indexes: [{ name: "by_org", fields: ["orgId"], unique: false }],
  },
);

// Pages live inside a Site. `slug` is per-site; "/" marks the home page.
// `sort` orders pages in the sidebar tree.
const Page = entity(
  "Page",
  {
    orgId: field.id("Organization"),
    siteId: field.id("Site"),
    slug: field.string(), // "/" | "about" | "pricing" …
    title: field.string(),
    sort: field.float(),
    metaTitle: field.string().optional(),
    metaDescription: field.string().optional(),
    createdAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_site_slug", fields: ["siteId", "slug"], unique: true },
      { name: "by_site_sort", fields: ["siteId", "sort"], unique: false },
    ],
  },
);

// Blocks are the editor's atoms. `type` picks the renderer; `propsJson`
// holds type-specific props (text, color, alignment, href, …). Keeping
// props as a JSON blob means new block types can ship without a schema
// migration — the client-side type registry controls what's valid.
//
// `parentId` enables Container nesting; top-level blocks have parentId=null.
// `componentId` is set on blocks that are instances of a Component — the
// renderer walks the component's master tree instead of reading local
// children. Editing the master updates every instance via sync.
//
// `propsJson` now supports two shapes:
//   - flat:   {"text":"x", "align":"left"}
//   - keyed:  {"desktop":{"text":"x"}, "tablet":{"align":"center"}}
// The renderer detects "desktop"/"tablet"/"phone" top-level keys and
// merges in order (desktop → current breakpoint). Flat shape is
// equivalent to desktop-only.
const Block = entity(
  "Block",
  {
    orgId: field.id("Organization"),
    siteId: field.id("Site"),
    // pageId is optional because component-master blocks aren't on a
    // concrete page — they live under a Component and render into
    // every instance. Regular canvas blocks always have pageId set.
    pageId: field.id("Page").optional(),
    parentId: field.id("Block").optional(),
    componentId: field.id("Component").optional(),
    sort: field.float(),
    type: field.string(), // heading | text | button | image | container | divider | component
    propsJson: field.string(), // JSON: shape depends on type
    createdAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_page_sort", fields: ["pageId", "sort"], unique: false },
      { name: "by_parent", fields: ["parentId"], unique: false },
      { name: "by_component", fields: ["componentId"], unique: false },
    ],
  },
);

// A Component is a reusable block subtree. Its master tree lives as a
// set of Blocks with `siteId=...` and `componentId=this.id`, no
// `pageId`. Instances reference it by componentId; the renderer walks
// the master tree to render. Edits to the master flow to every
// instance live via sync.
const Component = entity(
  "Component",
  {
    orgId: field.id("Organization"),
    siteId: field.id("Site"),
    name: field.string(),
    createdBy: field.id("User"),
    createdAt: field.datetime(),
  },
  {
    indexes: [{ name: "by_site", fields: ["siteId"], unique: false }],
  },
);

// Uploaded images / files. Kept minimal — the point is to demonstrate
// the shape, not build a full DAM.
const Asset = entity(
  "Asset",
  {
    orgId: field.id("Organization"),
    siteId: field.id("Site"),
    name: field.string(),
    kind: field.string(), // image | file
    url: field.string(), // data URL or external URL
    width: field.float().optional(),
    height: field.float().optional(),
    uploadedBy: field.id("User"),
    uploadedAt: field.datetime(),
  },
  {
    indexes: [{ name: "by_site", fields: ["siteId"], unique: false }],
  },
);

// ---------------------------------------------------------------------------
// Policies
// ---------------------------------------------------------------------------

const orgScoped = (name: string) =>
  policy({
    name: `${name}_org_scoped`,
    entity: name,
    allowRead: "auth.tenantId == data.orgId",
    allowInsert: "auth.tenantId == data.orgId",
    allowUpdate: "auth.tenantId == data.orgId",
    allowDelete: "auth.tenantId == data.orgId",
  });

// Sites are editor-readable to members. The /p/:slug public-preview route
// goes through a server-side function (`renderPublicSite`) running with
// admin auth — it lives outside this policy so anonymous visitors can
// read published sites without membership.
const siteReadable = policy({
  name: "site_readable",
  entity: "Site",
  allowRead: "auth.tenantId == data.orgId",
  allowInsert: "auth.tenantId == data.orgId",
  allowUpdate: "auth.tenantId == data.orgId",
  allowDelete: "auth.tenantId == data.orgId",
});

const organizationPolicy = policy({
  name: "organization_access",
  entity: "Organization",
  allowRead: "auth.userId != null",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId == data.createdBy",
  allowDelete: "auth.userId == data.createdBy",
});

const orgMemberPolicy = policy({
  name: "org_member_access",
  entity: "OrgMember",
  allowRead: "auth.userId == data.userId || auth.tenantId == data.orgId",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.tenantId == data.orgId",
  allowDelete: "auth.tenantId == data.orgId",
});

const manifest = buildManifest({
  name: "stage",
  version: "0.1.0",
  entities: [User, Organization, OrgMember, Site, Page, Block, Component, Asset],
  queries: [],
  actions: [],
  policies: [
    organizationPolicy,
    orgMemberPolicy,
    siteReadable,
    orgScoped("Page"),
    orgScoped("Block"),
    orgScoped("Component"),
    orgScoped("Asset"),
  ],
  routes: [],
});

console.log(JSON.stringify(manifest, null, 2));
