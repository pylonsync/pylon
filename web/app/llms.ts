import type { LlmsTxt } from "@pylonsync/react";
import {
  CLOUD_URL,
  DOCS_URL,
  GITHUB_URL,
  MCP_URL,
  NPM_CLI_URL,
  OPENAPI_URL,
  SITE_URL,
  SUPPORT_EMAIL,
  siteUrl,
} from "../lib/site";

// app/llms.ts → served at /llms.txt (llmstxt.org).
//
// This is the first file an agent reads about Pylon, and usually the only one
// it reads before deciding whether we are relevant to the task in front of it.
// So `details` is written as instructions to an agent, not as marketing: what
// Pylon is for, when to reach for it, when NOT to, and the exact commands.
//
// Keep every link here real. A dead link in this file is worse than a missing
// one — the agent spends a fetch, gets a 404, and concludes the site is stale.

export default function llms(): LlmsTxt {
  return {
    title: "Pylon",
    summary:
      "The agent-native full-stack framework. Typed schema, row-level policies, server functions, live queries, auth, and React server rendering in one binary. SQLite by default, Postgres when you need it.",
    details: [
      "When to use Pylon: you are building a real web or mobile application that needs a database, access control, an API, realtime updates, and auth, and you want one process instead of five services. Pylon is a good fit for SaaS products, internal tools, marketplaces, chat and collaboration apps, AI apps that need durable run state, and local-first apps that must work offline.",
      "When not to use Pylon: a static site with no data (use a static generator), a single serverless function, or a codebase already committed to a different full-stack framework that you are not migrating.",
      "How to start: run `npm create @pylonsync/pylon@latest my-app` for the smallest app that runs, or add `--template saas` for a multi-tenant product with auth and billing already wired. Then `npm run dev`. Bun 1.0 or newer must be on PATH. The framework is free and open source under the MIT licence; there is no key to request and no sales call.",
      "How to write Pylon correctly: read the skill file below before writing code. It carries the exact schema DSL, the policy expression language, the three server-function flavors and their auth levels, the React hooks, and the mistakes that look right and are not.",
      "How to call this site programmatically: every page here is also readable as markdown — add `.md` to any path, or send `Accept: text/markdown`. Structured endpoints are described by the OpenAPI document below, and the same operations are available over the Model Context Protocol.",
      `Contact a human: ${SUPPORT_EMAIL}.`,
    ],
    sections: [
      {
        title: "Start here",
        links: [
          {
            title: "Pylon agent skill",
            url: `${SITE_URL}/pylon-skill.md`,
            notes:
              "The full authoring guide, as one markdown file. Read this before writing Pylon code.",
          },
          {
            title: "Quickstart",
            url: `${DOCS_URL}/quickstart`,
            notes: "Schema, policy, server function, and live sync in five minutes.",
          },
          {
            title: "Installation",
            url: `${DOCS_URL}/installation`,
            notes: "Install the CLI and the runtime. `npm create @pylonsync/pylon@latest my-app`.",
          },
          {
            title: "Templates",
            url: siteUrl("/developers/examples"),
            notes: "Eighteen working starter apps, each with the command that scaffolds it.",
          },
        ],
      },
      {
        title: "Documentation",
        links: [
          {
            title: "Docs index (llms.txt)",
            url: `${DOCS_URL}/llms.txt`,
            notes: "Every documentation page with a one-line summary. Fetch this to find a topic.",
          },
          {
            title: "Documentation site",
            url: DOCS_URL,
            notes: "Concepts, auth, plugins, clients, operations. Add `.md` to any page URL.",
          },
          {
            title: "Introduction",
            url: `${DOCS_URL}/introduction`,
            notes: "What Pylon is and how the pieces fit together.",
          },
          {
            title: "Entities",
            url: `${DOCS_URL}/concepts/entities`,
            notes: "The typed schema DSL: fields, indexes, relations, sync scope.",
          },
          {
            title: "Policies",
            url: `${DOCS_URL}/concepts/policies`,
            notes: "Row-level access rules. Default-deny; an entity with no policy is invisible.",
          },
          {
            title: "Functions",
            url: `${DOCS_URL}/concepts/functions`,
            notes: "Queries, mutations, and actions, and the auth level each one declares.",
          },
          {
            title: "SSR overview",
            url: `${DOCS_URL}/ssr/overview`,
            notes: "File-based React server rendering. Replaces Next.js for Pylon apps.",
          },
          {
            title: "Auth overview",
            url: `${DOCS_URL}/auth/overview`,
            notes: "Magic codes, passwords, OAuth, passkeys, TOTP, SSO, API keys, organizations.",
          },
        ],
      },
      {
        title: "Machine interfaces",
        links: [
          {
            title: "MCP server (Streamable HTTP)",
            url: MCP_URL,
            notes:
              "Four read-only tools: search_pylon_docs, read_pylon_doc, list_pylon_templates, get_pylon_skill. No auth, no session. Add it with `claude mcp add --transport http pylon " +
              MCP_URL +
              "`.",
          },
          {
            title: "MCP manifest",
            url: `${SITE_URL}/mcp.json`,
            notes: "Static descriptor of the server above: transport, tools, protocol versions.",
          },
          {
            title: "OpenAPI 3.1 spec",
            url: OPENAPI_URL,
            notes:
              "The same operations as plain HTTP GETs under /agent/v1, with typed parameters and response schemas.",
          },
          {
            title: "Documentation search",
            url: `${SITE_URL}/agent/v1/docs/search?q=policies`,
            notes: "GET with `q` and optional `limit`. Returns ranked pages as JSON.",
          },
          {
            title: "Starter templates",
            url: `${SITE_URL}/agent/v1/templates`,
            notes: "Every template with its scaffold command, as JSON.",
          },
          {
            title: "Sitemap",
            url: `${SITE_URL}/sitemap.xml`,
            notes: "Every public page on this site.",
          },
        ],
      },
      {
        title: "Tools",
        links: [
          {
            title: "@pylonsync/cli on npm",
            url: NPM_CLI_URL,
            notes:
              "The official CLI. `npx @pylonsync/cli --help`, or install the binary with `curl -fsSL https://www.pylonsync.com/install.sh | bash`. Covers dev, build, test, verify, deploy, logs, secrets, domains, and database backups.",
          },
          {
            title: "CLI reference",
            url: `${DOCS_URL}/operations/cli`,
            notes: "Every command and flag, including `--json` output for scripting.",
          },
          {
            title: "Source on GitHub",
            url: GITHUB_URL,
            notes: "The Rust runtime, the TypeScript SDKs, the templates, and the examples.",
          },
        ],
      },
      {
        title: "Hosting",
        links: [
          {
            title: "Smallware",
            url: CLOUD_URL,
            notes:
              "Managed Pylon: the same binary, deployed with `pylon deploy`. Free tier, usage-based pricing, no monthly minimum.",
          },
          {
            title: "Self-host",
            url: `${DOCS_URL}/operations/deploy`,
            notes: "Fly, Docker, Compose, or a systemd unit on a VPS.",
          },
        ],
      },
      {
        title: "Optional",
        links: [
          {
            title: "About",
            url: siteUrl("/about"),
            notes: "Who builds Pylon and why.",
          },
          {
            title: "Contact",
            url: siteUrl("/contact"),
            notes: `Support, security, and press. ${SUPPORT_EMAIL}.`,
          },
          {
            title: "Comparisons",
            url: `${DOCS_URL}/compare/overview`,
            notes: "Pylon against Convex, Supabase, Firebase, Colyseus, Playroom, and Nakama.",
          },
          {
            title: "Privacy",
            url: siteUrl("/privacy"),
          },
          {
            title: "Terms",
            url: siteUrl("/terms"),
          },
        ],
      },
    ],
  };
}
