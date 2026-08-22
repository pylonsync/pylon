// One source for the URLs and contact details this site publishes about
// itself. They are repeated across page metadata, JSON-LD, llms.txt, the
// OpenAPI document, and the MCP manifest — and a machine reads all of those,
// so a disagreement between them is not a typo, it is contradictory data about
// who we are.
//
// `www` is deliberate and load-bearing. The apex 308-redirects to www, so an
// apex URL in a sitemap or a JSON-LD `url` sends every crawler through a
// redirect and leaves the canonical host ambiguous.

export const SITE_URL = "https://www.pylonsync.com";
export const DOCS_URL = "https://docs.pylonsync.com";
/** Managed Pylon. Different brand, different app, same framework. */
export const CLOUD_URL = "https://www.usesmallware.com";
export const GITHUB_URL = "https://github.com/pylonsync/pylon";
export const NPM_CLI_URL = "https://www.npmjs.com/package/@pylonsync/cli";
export const NPM_ORG_URL = "https://www.npmjs.com/org/pylonsync";
export const X_URL = "https://x.com/pylonsync";

/** Published support address. Also the JSON-LD `contactPoint`. */
export const SUPPORT_EMAIL = "support@pylonsync.com";
export const PRIVACY_EMAIL = "privacy@pylonsync.com";
export const LEGAL_EMAIL = "legal@pylonsync.com";

/** The MCP endpoint, Streamable HTTP transport. */
export const MCP_URL = `${SITE_URL}/mcp`;
/** OpenAPI 3.1 description of the public agent API. */
export const OPENAPI_URL = `${SITE_URL}/openapi.json`;
/** The read-only agent API's base path. */
export const AGENT_API_BASE = "/agent/v1";

/** Absolute URL for a site-relative path. */
export function siteUrl(path: string): string {
  if (!path.startsWith("/")) return `${SITE_URL}/${path}`;
  return path === "/" ? `${SITE_URL}/` : `${SITE_URL}${path}`;
}
