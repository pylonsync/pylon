// The marketing "products" now live in the single site config so the whole
// template can be rebranded from one file. This module re-exports them so
// existing imports (`@/lib/products`) keep working. Edit lib/site.config.ts.
export {
  PRODUCTS,
  productBySlug,
  type Product,
  type ProductFeature,
} from "./site.config";
