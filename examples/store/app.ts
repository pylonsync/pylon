/**
 * Pylon Store — faceted search showcase.
 *
 * 10,000-SKU demo storefront demonstrating Pylon's native faceted
 * full-text search: BM25 text match across name + description, live
 * facet counts for brand / category / color, price-range filters, and
 * sort-by-anything declared in `sortable`.
 *
 * The Product entity opts in to search via the `search:` block — that
 * single declaration is everything required. On schema push, Pylon
 * creates `_fts_Product` (FTS5 shadow) and the shared `_facet_bitmap`
 * table. Every insert/update/delete maintains both in the same
 * transaction, so `db.useSearch` in the UI reflects writes instantly.
 */
import { entity, field, policy, buildManifest } from "@pylonsync/sdk";

const Product = entity(
  "Product",
  {
    name: field.string(),
    description: field.richtext(),
    brand: field.string(),
    category: field.string(),
    color: field.string(),
    price: field.float(),
    rating: field.float(),
    stock: field.int(),
    imageUrl: field.string().optional(),
    createdAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_category", fields: ["category"], unique: false },
      { name: "by_brand", fields: ["brand"], unique: false },
      { name: "by_price", fields: ["price"], unique: false },
    ],
    search: {
      // BM25 match across these fields, weighted in declared order.
      text: ["name", "description"],
      // Facet bitmaps maintained for these columns — the UI renders
      // live counts beside each filter.
      facets: ["brand", "category", "color"],
      // Allowed sort keys. Anything outside this list is silently
      // dropped by the planner.
      sortable: ["price", "rating", "createdAt"],
    },
  },
);

// Public catalog — anyone can browse. Faceted search requires a
// row-independent read policy (row-scoped entities refuse search
// at the API layer to avoid leaking aggregate counts about rows
// the caller can't read).
const productPolicy = policy({
  name: "product_public",
  entity: "Product",
  allowRead: "true",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId != null",
  allowDelete: "auth.userId != null",
});

const manifest = buildManifest({
  name: "store",
  version: "0.1.0",
  entities: [Product],
  queries: [],
  actions: [],
  policies: [productPolicy],
  routes: [],
});

console.log(JSON.stringify(manifest, null, 2));
