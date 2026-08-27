// JSON-LD identity for pylonsync.com.
//
// Two things are being described, and conflating them is the usual mistake:
//   - Pylon, a `SoftwareApplication` — the framework someone installs.
//   - Pylon, the `Organization` that publishes it — who to contact.
// They are linked by `@id`, so a parser reads one graph and not two unrelated
// blobs that happen to share a name.
//
// Everything here must already be true on the page. Structured data that
// claims a rating, a price, or an address the site does not show is the thing
// search engines penalize, and the thing an agent repeats to a user.

import {
  CLOUD_URL,
  DOCS_URL,
  GITHUB_URL,
  NPM_CLI_URL,
  SITE_URL,
  SUPPORT_EMAIL,
  X_URL,
} from "../site";

const ORG_ID = `${SITE_URL}/#organization`;
const APP_ID = `${SITE_URL}/#software`;
const SITE_ID = `${SITE_URL}/#website`;

/**
 * The publisher.
 *
 * `address` is country-level. schema.org does not require a street on a
 * PostalAddress, and publishing a street that is a home address to satisfy a
 * checklist is a bad trade. Add locality/region here when there is a real
 * business address to give.
 */
export function organizationLd() {
  return {
    "@type": "Organization",
    "@id": ORG_ID,
    name: "Pylon",
    alternateName: "Pylonsync",
    url: `${SITE_URL}/`,
    logo: {
      "@type": "ImageObject",
      url: `${SITE_URL}/brand/pylon-logo@2x.png`,
    },
    description:
      "Pylon builds the agent-native full-stack framework of the same name, and Stack0 Cloud, its managed hosting.",
    email: SUPPORT_EMAIL,
    contactPoint: [
      {
        "@type": "ContactPoint",
        contactType: "customer support",
        email: SUPPORT_EMAIL,
        url: `${SITE_URL}/contact`,
        availableLanguage: ["English"],
      },
      {
        "@type": "ContactPoint",
        contactType: "technical support",
        email: SUPPORT_EMAIL,
        url: `${GITHUB_URL}/issues`,
        availableLanguage: ["English"],
      },
    ],
    address: {
      "@type": "PostalAddress",
      addressCountry: "US",
    },
    sameAs: [GITHUB_URL, X_URL, NPM_CLI_URL, CLOUD_URL],
  };
}

/** The framework itself. Free and open source, so `offers` is a zero price. */
export function softwareApplicationLd() {
  return {
    "@type": "SoftwareApplication",
    "@id": APP_ID,
    name: "Pylon",
    applicationCategory: "DeveloperApplication",
    applicationSubCategory: "Web application framework",
    operatingSystem: "macOS, Linux",
    url: `${SITE_URL}/`,
    downloadUrl: NPM_CLI_URL,
    installUrl: `${SITE_URL}/install.sh`,
    softwareHelp: { "@type": "CreativeWork", url: DOCS_URL },
    codeRepository: GITHUB_URL,
    programmingLanguage: ["TypeScript", "Rust"],
    license: "https://opensource.org/license/mit",
    description:
      "Pylon is a full-stack framework built for coding agents: a typed schema, row-level policies, server functions, live queries, auth, and React server rendering in one binary. SQLite by default, Postgres when you need it.",
    featureList: [
      "Typed schema with automatic migrations",
      "Row-level access policies",
      "Server functions: queries, mutations, actions",
      "Live queries over WebSocket",
      "Built-in auth: magic codes, OAuth, passkeys, TOTP, SSO",
      "React server rendering with file-based routing",
      "Full-text and vector search",
      "One-command deploy",
    ],
    offers: {
      "@type": "Offer",
      price: "0",
      priceCurrency: "USD",
      availability: "https://schema.org/InStock",
      description: "The framework is free and open source under the MIT licence.",
    },
    publisher: { "@id": ORG_ID },
    author: { "@id": ORG_ID },
  };
}

/** The site, so a parser can tie pages to the publisher. */
export function webSiteLd() {
  return {
    "@type": "WebSite",
    "@id": SITE_ID,
    url: `${SITE_URL}/`,
    name: "Pylon",
    description:
      "Pylon is a full-stack framework built for agents to ship high-performance and secure apps quickly.",
    inLanguage: "en",
    publisher: { "@id": ORG_ID },
  };
}

/** One `WebPage` node, for pages that describe the organization itself. */
export function webPageLd(args: { path: string; name: string; description: string }) {
  const url = args.path === "/" ? `${SITE_URL}/` : `${SITE_URL}${args.path}`;
  return {
    "@type": "WebPage",
    "@id": `${url}#webpage`,
    url,
    name: args.name,
    description: args.description,
    isPartOf: { "@id": SITE_ID },
    about: { "@id": ORG_ID },
    publisher: { "@id": ORG_ID },
  };
}

/** The homepage graph: site, publisher, product. */
export function homepageGraph() {
  return {
    "@context": "https://schema.org",
    "@graph": [webSiteLd(), organizationLd(), softwareApplicationLd()],
  };
}

/** The graph for /about and /contact: site, publisher, and this page. */
export function organizationPageGraph(args: {
  path: string;
  name: string;
  description: string;
}) {
  return {
    "@context": "https://schema.org",
    "@graph": [webSiteLd(), organizationLd(), webPageLd(args)],
  };
}

/**
 * Serialize a graph for a `<script type="application/ld+json">` body.
 *
 * `<` is escaped so a value containing `</script` cannot close the tag early —
 * the standard XSS hole in hand-rolled JSON-LD blocks. Every value here is a
 * constant today; the escape is what keeps that from mattering later.
 */
export function serializeJsonLd(graph: unknown): string {
  return JSON.stringify(graph).replace(/</g, "\\u003c");
}
