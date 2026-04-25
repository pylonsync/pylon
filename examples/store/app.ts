/**
 * Pylon Store — full e-commerce showcase.
 *
 * Demonstrates Pylon's full surface area in one example:
 *   - Faceted full-text search (Product)
 *   - Per-user sync-backed cart (CartItem)
 *   - Shipping addresses (Address)
 *   - Orders + order items (Order, OrderItem)
 *   - Server-side checkout function with atomic cart-clear
 *   - Scheduled status progression (placed → packed → shipped → delivered)
 *
 * Auth uses Pylon's password endpoints (`/api/auth/password/register`,
 * `/api/auth/password/login`) plus guest sessions for anonymous browsing.
 */
import { entity, field, policy, buildManifest } from "@pylonsync/sdk";

// User row backing the password endpoints. The fields below match what
// `/api/auth/password/register` writes; auth itself is handled by the
// framework, so this file is just a manifest declaration.
const User = entity(
  "User",
  {
    email: field.string(),
    displayName: field.string(),
    avatarColor: field.string().optional(),
    passwordHash: field.string().optional(),
    createdAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_email", fields: ["email"], unique: true },
    ],
  },
);

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
      text: ["name", "description"],
      facets: ["brand", "category", "color"],
      sortable: ["price", "rating", "createdAt"],
    },
  },
);

// Per-user cart. Sync-backed so items persist across reloads and
// multiple tabs stay in lockstep without polling.
const CartItem = entity(
  "CartItem",
  {
    userId: field.string(),
    productId: field.string(),
    productName: field.string(),
    productBrand: field.string(),
    productPrice: field.float(),
    quantity: field.int(),
    addedAt: field.datetime(),
  },
  {
    indexes: [{ name: "by_user", fields: ["userId"], unique: false }],
  },
);

// Shipping addresses. A user can have multiple; one is marked default.
const Address = entity(
  "Address",
  {
    userId: field.string(),
    fullName: field.string(),
    street: field.string(),
    city: field.string(),
    postal: field.string(),
    country: field.string(),
    isDefault: field.bool(),
  },
  {
    indexes: [{ name: "by_user", fields: ["userId"], unique: false }],
  },
);

// An Order snapshots its shipping address so future Address edits
// don't rewrite history. `status` advances via scheduled function:
//   placed (immediate) → packed (+15s) → shipped (+30s) → delivered (+90s)
// Demo timings — production would be hours/days, not seconds.
const Order = entity(
  "Order",
  {
    userId: field.string(),
    status: field.string(),
    subtotal: field.float(),
    itemCount: field.int(),
    shipName: field.string(),
    shipStreet: field.string(),
    shipCity: field.string(),
    shipPostal: field.string(),
    shipCountry: field.string(),
    placedAt: field.datetime(),
    trackingNumber: field.string(),
    estimatedDelivery: field.datetime(),
  },
  {
    indexes: [
      { name: "by_user", fields: ["userId"], unique: false },
      { name: "by_user_placed", fields: ["userId", "placedAt"], unique: false },
    ],
  },
);

// Line items for an order. Snapshotted at placement so price changes
// to the catalog don't retroactively rewrite past orders.
const OrderItem = entity(
  "OrderItem",
  {
    orderId: field.string(),
    userId: field.string(),
    productId: field.string(),
    productName: field.string(),
    productBrand: field.string(),
    unitPrice: field.float(),
    quantity: field.int(),
  },
  {
    indexes: [{ name: "by_order", fields: ["orderId"], unique: false }],
  },
);

// ---------------------------------------------------------------------------
// Policies
// ---------------------------------------------------------------------------

// Users can read User rows when authenticated. Clients are expected
// to scope queries by id; this policy only gates the auth check.
// Writes go exclusively through the password endpoints — never
// directly via /api/entities/User — so insert and update are denied.
const userPolicy = policy({
  name: "user_self",
  entity: "User",
  allowRead: "auth.userId != null",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// Public catalog. Faceted search requires row-independent reads.
const productPolicy = policy({
  name: "product_public",
  entity: "Product",
  allowRead: "true",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId != null",
  allowDelete: "auth.userId != null",
});

// Convention: per-user entities use `allowRead: auth.userId != null` and
// the client always queries with `where: { userId }`. Writes use
// `data.userId == auth.userId` to prevent impersonation. Same shape as
// the other examples in this repo (see `trade/app.ts` for prior art).
const cartPolicy = policy({
  name: "cart_owner",
  entity: "CartItem",
  allowRead: "auth.userId != null",
  allowInsert: "auth.userId == data.userId",
  allowUpdate: "auth.userId != null",
  allowDelete: "auth.userId != null",
});

const addressPolicy = policy({
  name: "address_owner",
  entity: "Address",
  allowRead: "auth.userId != null",
  allowInsert: "auth.userId == data.userId",
  allowUpdate: "auth.userId != null",
  allowDelete: "auth.userId != null",
});

// Orders + items: writes go through the `placeOrder` /
// `advanceOrderStatus` server functions only; the API never inserts
// directly. Reads are gated to authenticated users and the client
// scopes by userId.
const orderPolicy = policy({
  name: "order_owner",
  entity: "Order",
  allowRead: "auth.userId != null",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const orderItemPolicy = policy({
  name: "order_item_owner",
  entity: "OrderItem",
  allowRead: "auth.userId != null",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const manifest = buildManifest({
  name: "store",
  version: "0.1.0",
  entities: [User, Product, CartItem, Address, Order, OrderItem],
  queries: [],
  actions: [],
  policies: [
    userPolicy,
    productPolicy,
    cartPolicy,
    addressPolicy,
    orderPolicy,
    orderItemPolicy,
  ],
  routes: [],
});

console.log(JSON.stringify(manifest, null, 2));
