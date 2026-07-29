// Re-exports the marketing products from the single site config so existing
// `@/lib/products` imports keep working. Edit lib/site.config.ts.
export {
  PRODUCTS,
  productBySlug,
  type Product,
  type ProductFeature,
} from "./site.config";
