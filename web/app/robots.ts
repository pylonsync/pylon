type Robots = {
	rules: {
		userAgent: string;
		allow?: string | string[];
		disallow?: string | string[];
	};
	sitemap?: string;
};

// app/robots.ts → served at /robots.txt. Canonical host pylonsync.com; the
// cloud API/dashboard paths are kept out of the index.
const SITE = "https://pylonsync.com";

export default function robots(): Robots {
  return {
    rules: {
      userAgent: "*",
      allow: "/",
      disallow: [
        "/dashboard",
        "/admin",
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
