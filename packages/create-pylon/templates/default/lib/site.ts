// Re-exports the marketing content (solutions, resources, company, comparisons)
// from the single site config so existing `@/lib/site` imports keep working.
// Edit lib/site.config.ts.
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
