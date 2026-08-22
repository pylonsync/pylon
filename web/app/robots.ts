import { SITE_URL } from "../lib/site";

type Robots = {
	rules: {
		userAgent: string;
		allow?: string | string[];
		disallow?: string | string[];
	};
	sitemap?: string;
};

// app/robots.ts → served at /robots.txt. Canonical host is www.pylonsync.com;
// the cloud API/dashboard paths are kept out of the index.
const SITE = SITE_URL;

export default function robots(): Robots {
  return {
    rules: {
      userAgent: "*",
      // `/agent/` is named explicitly even though `/` already allows it: the
      // read-only agent API sits one path segment away from the disallowed
      // `/api/`, and an allow rule that says so out loud is cheaper than
      // finding out a crawler generalized.
      allow: ["/", "/agent/", "/llms.txt", "/openapi.json", "/mcp.json"],
      disallow: [
        "/dashboard",
        "/admin",
        // The framework's own RPC surface (`/api/fn/*`, `/api/entities/*`).
        // The public agent API is at /agent/v1 precisely so it is not caught
        // by this rule.
        "/api/",
        "/studio",
        "/status",
        "/onboarding",
        "/verify-email",
        "/github",
        "/invite/",
      ],
    },
    sitemap: `${SITE}/sitemap.xml`,
  };
}
