// The marketing content (solutions, resources, company, comparisons) now lives
// in the single site config so the whole template can be rebranded from one
// file. This module re-exports it so existing imports (`@/lib/site`) keep
// working. Edit lib/site.config.ts.
export {
  SOLUTIONS,
  RESOURCES,
  COMPANY,
  COMPARISONS,
  bySlug,
  type ContentSection,
  type SitePage,
  type Comparison,
} from "./site.config";
