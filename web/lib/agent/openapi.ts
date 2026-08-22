// OpenAPI 3.1 description of the public agent API, served at /openapi.json.
//
// Written to be consumed by an LLM function-calling layer, which needs three
// things this document is shaped around:
//   - a unique `operationId` per operation (it becomes the function name),
//   - a `description` on every operation AND every parameter (it is the only
//     thing the model reads when deciding what to send), and
//   - a real response schema (so a caller can type the result instead of
//     guessing at the JSON).
//
// Every path here is implemented by a `route.ts` under `web/app/agent/` or
// `web/app/mcp/`. Adding an endpoint without adding it here makes this file a
// lie, which is worse than not publishing it.

import { SITE_URL, SUPPORT_EMAIL } from "../site";
import { TOOLS } from "./mcp";

const DOC_HIT_SCHEMA = {
  type: "object",
  required: ["title", "url", "markdownUrl", "path"],
  properties: {
    title: { type: "string", description: "Page title." },
    url: { type: "string", format: "uri", description: "The human documentation page." },
    markdownUrl: {
      type: "string",
      format: "uri",
      description: "The same page as markdown. Fetch this one to read it.",
    },
    path: {
      type: "string",
      description: 'Documentation path, e.g. "concepts/policies". Pass it to /agent/v1/docs/read.',
    },
    summary: { type: "string", description: "One-line summary of the page." },
  },
} as const;

const TEMPLATE_SCHEMA = {
  type: "object",
  required: ["template", "name", "blurb", "shows", "command", "source"],
  properties: {
    template: { type: "string", description: 'The --template value, e.g. "saas".' },
    name: { type: "string", description: "Human name of the template." },
    blurb: { type: "string", description: "What the template contains." },
    shows: {
      type: "array",
      items: { type: "string" },
      description: "Pylon features this template exercises.",
    },
    command: { type: "string", description: "The exact command that scaffolds it." },
    source: { type: "string", format: "uri", description: "Template source on GitHub." },
    demo: { type: "string", format: "uri", description: "A deployed demo, when one exists." },
  },
} as const;

const ERROR_SCHEMA = {
  type: "object",
  required: ["error"],
  properties: {
    error: {
      type: "object",
      required: ["code", "message"],
      properties: {
        code: { type: "string", description: "Stable error code." },
        message: { type: "string", description: "What went wrong, in one sentence." },
      },
    },
  },
} as const;

function jsonResponse(description: string, schema: unknown) {
  return { description, content: { "application/json": { schema } } };
}

export function openApiDocument() {
  return {
    openapi: "3.1.0",
    info: {
      title: "Pylon agent API",
      version: "1.0.0",
      summary: "Read-only endpoints an agent can call while writing Pylon code.",
      description: [
        "Search and read the Pylon documentation, list the starter templates, and fetch the",
        "agent authoring guide. Every endpoint is public, read-only, unauthenticated, and",
        "rate-limited by the framework's own limiter — there is no key to request and no",
        "form to fill in.",
        "",
        "The same four operations are available over MCP (Streamable HTTP) at " + `${SITE_URL}/mcp.`,
      ].join("\n"),
      contact: { name: "Pylon", email: SUPPORT_EMAIL, url: `${SITE_URL}/contact` },
      license: { name: "MIT", identifier: "MIT" },
    },
    servers: [{ url: SITE_URL, description: "Production" }],
    externalDocs: { description: "Pylon documentation", url: "https://docs.pylonsync.com" },
    paths: {
      "/agent/v1/docs": {
        get: {
          operationId: "listPylonDocs",
          summary: "List every Pylon documentation page",
          description:
            "Return the full documentation index: title, path, one-line summary, and the markdown URL for each page. Use searchPylonDocs when you have a keyword; use this when you want the whole map.",
          tags: ["docs"],
          responses: {
            "200": jsonResponse("The documentation index.", {
              type: "object",
              required: ["count", "results"],
              properties: {
                count: { type: "integer", description: "Number of pages returned." },
                results: { type: "array", items: DOC_HIT_SCHEMA },
              },
            }),
          },
        },
      },
      "/agent/v1/docs/search": {
        get: {
          operationId: "searchPylonDocs",
          summary: "Search the Pylon documentation",
          description:
            "Search documentation page titles, paths, and summaries by keyword. Call this before answering a question about how Pylon works, then read the winning page with readPylonDoc.",
          tags: ["docs"],
          parameters: [
            {
              name: "q",
              in: "query",
              required: true,
              description: 'Keywords to match. Example: "row level policy" or "vector search".',
              schema: { type: "string", minLength: 1 },
            },
            {
              name: "limit",
              in: "query",
              required: false,
              description: "Maximum number of pages to return.",
              schema: { type: "integer", minimum: 1, maximum: 50, default: 10 },
            },
          ],
          responses: {
            "200": jsonResponse("Ranked matches, best first.", {
              type: "object",
              required: ["query", "count", "results"],
              properties: {
                query: { type: "string", description: "The query that was run." },
                count: { type: "integer", description: "Number of matches returned." },
                results: { type: "array", items: DOC_HIT_SCHEMA },
              },
            }),
            "400": jsonResponse("The `q` parameter was missing or empty.", ERROR_SCHEMA),
          },
        },
      },
      "/agent/v1/docs/read": {
        get: {
          operationId: "readPylonDoc",
          summary: "Read one documentation page as markdown",
          description:
            "Fetch a single documentation page as markdown. `path` must be one returned by searchPylonDocs or listPylonDocs; anything else is rejected.",
          tags: ["docs"],
          parameters: [
            {
              name: "path",
              in: "query",
              required: true,
              description:
                'Documentation path with no leading slash and no .md suffix, e.g. "concepts/entities".',
              schema: { type: "string", minLength: 1 },
            },
          ],
          responses: {
            "200": {
              description: "The page, as markdown.",
              content: {
                "text/markdown": { schema: { type: "string", description: "Markdown source." } },
              },
            },
            "400": jsonResponse("The `path` parameter was missing.", ERROR_SCHEMA),
            "404": jsonResponse("No such documentation page.", ERROR_SCHEMA),
          },
        },
      },
      "/agent/v1/templates": {
        get: {
          operationId: "listPylonTemplates",
          summary: "List the Pylon starter templates",
          description:
            "Return every create-pylon starter template with the exact scaffold command, the features it exercises, its source, and a live demo where one exists. Use this when a user asks to start a new Pylon app.",
          tags: ["templates"],
          parameters: [
            {
              name: "q",
              in: "query",
              required: false,
              description: "Optional keywords to filter by name, description, or feature.",
              schema: { type: "string" },
            },
          ],
          responses: {
            "200": jsonResponse("The matching templates.", {
              type: "object",
              required: ["count", "templates"],
              properties: {
                count: { type: "integer", description: "Number of templates returned." },
                templates: { type: "array", items: TEMPLATE_SCHEMA },
              },
            }),
          },
        },
      },
      "/agent/v1/skill": {
        get: {
          operationId: "getPylonSkill",
          summary: "Get the Pylon authoring guide",
          description:
            "Return the full Pylon agent skill as markdown: schema DSL, policy language, server functions, React hooks, SSR conventions, deployment, and the known footguns. About 60KB. Read it before writing a Pylon app from scratch.",
          tags: ["skill"],
          responses: {
            "200": {
              description: "The skill document.",
              content: {
                "text/markdown": { schema: { type: "string", description: "Markdown source." } },
              },
            },
          },
        },
      },
      "/mcp": {
        post: {
          operationId: "callPylonMcp",
          summary: "MCP endpoint (Streamable HTTP)",
          description: [
            "Model Context Protocol endpoint. Send one JSON-RPC 2.0 message per request with",
            "`Accept: application/json, text/event-stream`. Supported methods: initialize,",
            "server/discover, notifications/initialized, ping, tools/list, tools/call.",
            "The server is stateless — it issues no session id and needs none back.",
            "Tools: " + TOOLS.map((t) => t.name).join(", ") + ".",
          ].join(" "),
          tags: ["mcp"],
          requestBody: {
            required: true,
            content: {
              "application/json": {
                schema: {
                  type: "object",
                  required: ["jsonrpc", "method"],
                  properties: {
                    jsonrpc: { const: "2.0", description: 'Always "2.0".' },
                    id: {
                      description:
                        "Request id. Omit it for a notification, which answers 202 with no body.",
                      anyOf: [{ type: "string" }, { type: "integer" }],
                    },
                    method: { type: "string", description: "The JSON-RPC method." },
                    params: { type: "object", description: "Method parameters." },
                  },
                },
              },
            },
          },
          responses: {
            "200": jsonResponse("A JSON-RPC response.", {
              type: "object",
              required: ["jsonrpc", "id"],
              properties: {
                jsonrpc: { const: "2.0" },
                id: { anyOf: [{ type: "string" }, { type: "integer" }, { type: "null" }] },
                result: { type: "object", description: "Present on success." },
                error: {
                  type: "object",
                  description: "Present on failure.",
                  required: ["code", "message"],
                  properties: {
                    code: { type: "integer" },
                    message: { type: "string" },
                  },
                },
              },
            }),
            "202": { description: "A notification was accepted. No body." },
            "400": jsonResponse("Malformed JSON, or an unsupported MCP-Protocol-Version.", {
              type: "object",
              properties: {
                jsonrpc: { const: "2.0" },
                id: { type: "null" },
                error: { type: "object" },
              },
            }),
          },
        },
        get: {
          operationId: "openPylonMcpStream",
          summary: "Server-initiated stream (not offered)",
          description:
            "This server has nothing to push, so it does not open an SSE stream here. Answers 405 with an Allow header, as the transport specifies.",
          tags: ["mcp"],
          responses: { "405": { description: "No server-initiated stream at this endpoint." } },
        },
      },
    },
    tags: [
      { name: "docs", description: "Search and read the Pylon documentation." },
      { name: "templates", description: "Starter templates and their scaffold commands." },
      { name: "skill", description: "The agent authoring guide." },
      { name: "mcp", description: "The Model Context Protocol endpoint." },
    ],
  };
}
