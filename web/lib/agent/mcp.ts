// MCP server for pylonsync.com — JSON-RPC dispatch, transport-independent.
//
// The HTTP shell lives in `web/app/mcp/route.ts`; everything that decides what
// to answer lives here so it can be tested without a socket.
//
// Scope: one stateless server exposing four read-only tools over Streamable
// HTTP. No sessions (nothing to keep between calls), no SSE (every tool answers
// in one shot), no auth (the underlying content is public). Each of those is a
// deliberate "not needed here", not a gap — a session id an agent must echo
// back, for a server with no per-session state, is a way to fail, not a feature.
//
// Spec: https://modelcontextprotocol.io/specification/2026-07-28
//   - `initialize` + `notifications/initialized` — the handshake used by
//     2025-03-26 … 2025-11-25 clients, still what Claude Code and the OpenAI
//     Agents SDK send today.
//   - `server/discover` — the handshake-free entry point added in 2026-07-28.
//   - `tools/list`, `tools/call`, `ping`.

import {
  fetchDoc,
  listTemplates,
  readSkill,
  searchDocs,
  type DocHit,
} from "./tools";
import { SITE_URL } from "../site";

/** Protocol revisions this server answers, newest first. */
export const SUPPORTED_PROTOCOL_VERSIONS = [
  "2026-07-28",
  "2025-11-25",
  "2025-06-18",
  "2025-03-26",
  "2024-11-05",
] as const;

export const LATEST_PROTOCOL_VERSION = SUPPORTED_PROTOCOL_VERSIONS[0];

export const SERVER_INFO = {
  name: "pylonsync",
  title: "Pylon",
  version: "1.0.0",
  websiteUrl: SITE_URL,
} as const;

const INSTRUCTIONS = [
  "Pylon is a full-stack framework: typed schema, row-level policies, server functions,",
  "live queries, auth, and React SSR in one Rust binary that runs your TypeScript on Bun.",
  "Use these tools when you are writing or reviewing Pylon code, choosing a starter template,",
  "or answering a question about how Pylon works. Call search_pylon_docs first, then",
  "read_pylon_doc on the path it returns. get_pylon_skill returns the full authoring guide",
  "(about 60KB) — read it before writing a Pylon app from scratch.",
].join(" ");

export interface JsonRpcRequest {
  jsonrpc: "2.0";
  id?: string | number | null;
  method: string;
  params?: Record<string, unknown>;
}

export interface JsonRpcResponse {
  jsonrpc: "2.0";
  id: string | number | null;
  result?: unknown;
  error?: { code: number; message: string; data?: unknown };
}

// JSON-RPC 2.0 error codes, plus the MCP-specific ones we use.
export const PARSE_ERROR = -32700;
export const INVALID_REQUEST = -32600;
export const METHOD_NOT_FOUND = -32601;
export const INVALID_PARAMS = -32602;
export const INTERNAL_ERROR = -32603;

export interface ToolDefinition {
  name: string;
  title: string;
  description: string;
  inputSchema: Record<string, unknown>;
}

/**
 * The tools, with schemas an LLM can fill in without guessing. Every field
 * carries a description, because the description is the only thing the model
 * reads when deciding what to pass.
 */
export const TOOLS: ToolDefinition[] = [
  {
    name: "search_pylon_docs",
    title: "Search Pylon documentation",
    description:
      "Search the Pylon documentation index by keyword. Returns matching pages with their titles, one-line summaries, and both the human URL and the markdown URL. Use this before answering any question about how Pylon works, then read the page with read_pylon_doc.",
    inputSchema: {
      type: "object",
      properties: {
        query: {
          type: "string",
          description:
            "Keywords to match against page titles, paths, and summaries. Example: \"row level policy\" or \"vector search\".",
        },
        limit: {
          type: "integer",
          description: "Maximum number of pages to return. Default 10, maximum 50.",
          minimum: 1,
          maximum: 50,
          default: 10,
        },
      },
      required: ["query"],
      additionalProperties: false,
    },
  },
  {
    name: "read_pylon_doc",
    title: "Read a Pylon documentation page",
    description:
      "Fetch one Pylon documentation page as markdown. Pass a path returned by search_pylon_docs, such as \"concepts/policies\" or \"auth/magic-codes\". Paths that are not in the documentation index are rejected.",
    inputSchema: {
      type: "object",
      properties: {
        path: {
          type: "string",
          description:
            "Documentation path without a leading slash and without the .md suffix, e.g. \"concepts/entities\".",
        },
      },
      required: ["path"],
      additionalProperties: false,
    },
  },
  {
    name: "list_pylon_templates",
    title: "List Pylon starter templates",
    description:
      "List the create-pylon starter templates with the exact scaffold command, the features each one exercises, its source on GitHub, and a live demo where one exists. Use this when a user asks to start a new Pylon app, instead of writing a project from scratch.",
    inputSchema: {
      type: "object",
      properties: {
        query: {
          type: "string",
          description:
            "Optional keywords to filter templates by name, description, or the features they show.",
        },
      },
      additionalProperties: false,
    },
  },
  {
    name: "get_pylon_skill",
    title: "Get the Pylon authoring guide",
    description:
      "Return the full Pylon agent skill as markdown: the schema DSL, the policy language, the three server-function flavors, the React client hooks, SSR conventions, deployment, and the known footguns. About 60KB. Read it before writing a Pylon app from scratch.",
    inputSchema: { type: "object", properties: {}, additionalProperties: false },
  },
];

/** A tool result in MCP's content-block shape. */
function textResult(text: string, structured?: Record<string, unknown>) {
  return {
    content: [{ type: "text", text }],
    ...(structured ? { structuredContent: structured } : {}),
  };
}

function errorResult(message: string) {
  return { content: [{ type: "text", text: message }], isError: true };
}

function formatHits(hits: DocHit[]): string {
  if (hits.length === 0) return "No matching documentation pages.";
  return hits
    .map((h) => `- ${h.title} (path: ${h.path})${h.summary ? `\n  ${h.summary}` : ""}\n  ${h.url}`)
    .join("\n");
}

/** Run one tool call. Tool failures come back as results, not JSON-RPC errors. */
export async function callTool(
  name: string,
  args: Record<string, unknown>,
): Promise<Record<string, unknown>> {
  switch (name) {
    case "search_pylon_docs": {
      const query = typeof args.query === "string" ? args.query.trim() : "";
      if (!query) return errorResult("search_pylon_docs needs a non-empty `query`.");
      const limit = typeof args.limit === "number" ? args.limit : 10;
      const hits = await searchDocs(query, limit);
      return textResult(formatHits(hits), { results: hits });
    }
    case "read_pylon_doc": {
      const path = typeof args.path === "string" ? args.path.trim() : "";
      if (!path) return errorResult("read_pylon_doc needs a `path`.");
      const doc = await fetchDoc(path);
      if (!doc.ok) return errorResult(doc.error);
      return textResult(doc.markdown, { path: doc.path, url: doc.url });
    }
    case "list_pylon_templates": {
      const query = typeof args.query === "string" ? args.query.toLowerCase().trim() : "";
      let templates = listTemplates();
      if (query) {
        const terms = query.split(/\s+/).filter(Boolean);
        templates = templates.filter((t) => {
          const haystack = `${t.template} ${t.name} ${t.blurb} ${t.shows.join(" ")}`.toLowerCase();
          return terms.every((term) => haystack.includes(term));
        });
      }
      const text =
        templates.length === 0
          ? "No matching templates."
          : templates
              .map(
                (t) =>
                  `- ${t.name} (--template ${t.template})\n  ${t.blurb}\n  ${t.command}${t.demo ? `\n  demo: ${t.demo}` : ""}`,
              )
              .join("\n");
      return textResult(text, { templates });
    }
    case "get_pylon_skill": {
      const skill = await readSkill();
      return textResult(skill.markdown, { url: skill.url });
    }
    default:
      return errorResult(`Unknown tool "${name}".`);
  }
}

/** The capability + identity block, shared by `initialize` and `server/discover`. */
function serverDescriptor(protocolVersion: string) {
  return {
    protocolVersion,
    capabilities: { tools: { listChanged: false } },
    serverInfo: SERVER_INFO,
    instructions: INSTRUCTIONS,
  };
}

function ok(id: string | number | null, result: unknown): JsonRpcResponse {
  return { jsonrpc: "2.0", id, result };
}

function fail(
  id: string | number | null,
  code: number,
  message: string,
  data?: unknown,
): JsonRpcResponse {
  return { jsonrpc: "2.0", id, error: { code, message, ...(data ? { data } : {}) } };
}

/**
 * Dispatch one JSON-RPC message.
 *
 * Returns `null` for a notification (no `id`), which the transport turns into
 * `202 Accepted` with no body — the spec's shape for "received, nothing to
 * say back".
 */
export async function dispatch(message: unknown): Promise<JsonRpcResponse | null> {
  if (typeof message !== "object" || message === null || Array.isArray(message)) {
    return fail(null, INVALID_REQUEST, "Expected a single JSON-RPC request object.");
  }
  const req = message as JsonRpcRequest;
  const id = req.id ?? null;
  const isNotification = req.id === undefined || req.id === null;
  if (req.jsonrpc !== "2.0" || typeof req.method !== "string") {
    return isNotification
      ? null
      : fail(id, INVALID_REQUEST, 'A JSON-RPC request needs "jsonrpc": "2.0" and a "method".');
  }
  const params = (req.params ?? {}) as Record<string, unknown>;

  switch (req.method) {
    case "initialize": {
      // Negotiate: answer in the client's version when we speak it, otherwise
      // in ours. The client then decides whether to continue.
      const asked = typeof params.protocolVersion === "string" ? params.protocolVersion : "";
      const version = (SUPPORTED_PROTOCOL_VERSIONS as readonly string[]).includes(asked)
        ? asked
        : LATEST_PROTOCOL_VERSION;
      return ok(id, serverDescriptor(version));
    }
    case "server/discover": {
      // 2026-07-28: one request returns identity, capabilities, and every
      // protocol version we speak — no handshake required.
      return ok(id, {
        ...serverDescriptor(LATEST_PROTOCOL_VERSION),
        protocolVersions: [...SUPPORTED_PROTOCOL_VERSIONS],
      });
    }
    case "notifications/initialized":
    case "initialized":
      return null;
    case "ping":
      return isNotification ? null : ok(id, {});
    case "tools/list":
      return ok(id, { tools: TOOLS });
    case "tools/call": {
      const name = typeof params.name === "string" ? params.name : "";
      const args =
        typeof params.arguments === "object" && params.arguments !== null
          ? (params.arguments as Record<string, unknown>)
          : {};
      if (!name) return fail(id, INVALID_PARAMS, "tools/call needs a tool `name`.");
      if (!TOOLS.some((t) => t.name === name)) {
        return fail(id, INVALID_PARAMS, `Unknown tool "${name}". Call tools/list first.`);
      }
      try {
        return ok(id, await callTool(name, args));
      } catch (e) {
        // A tool that threw is our bug, not a protocol error the client can
        // fix — but it still must come back as a well-formed response.
        return fail(id, INTERNAL_ERROR, `Tool "${name}" failed: ${(e as Error).message}`);
      }
    }
    // Declared unsupported rather than silently empty: a client that asks is
    // told no, and knows not to ask again.
    case "resources/list":
    case "resources/templates/list":
    case "prompts/list":
      return fail(id, METHOD_NOT_FOUND, `This server exposes tools only ("${req.method}" is not implemented).`);
    default:
      return isNotification ? null : fail(id, METHOD_NOT_FOUND, `Unknown method "${req.method}".`);
  }
}

/** Is this protocol version one we answer? Used for the header check. */
export function isSupportedProtocolVersion(version: string): boolean {
  return (SUPPORTED_PROTOCOL_VERSIONS as readonly string[]).includes(version);
}

/**
 * The discovery manifest served at `/mcp.json`, for clients and directories
 * that read a static descriptor before connecting.
 */
export function mcpManifest() {
  return {
    name: SERVER_INFO.name,
    title: SERVER_INFO.title,
    description:
      "Read-only MCP server for the Pylon framework: search and read the docs, list starter templates, and fetch the agent authoring guide.",
    version: SERVER_INFO.version,
    websiteUrl: SITE_URL,
    remotes: [{ type: "streamable-http", url: `${SITE_URL}/mcp` }],
    capabilities: { tools: true, resources: false, prompts: false },
    authentication: { type: "none" },
    protocolVersions: [...SUPPORTED_PROTOCOL_VERSIONS],
    tools: TOOLS.map((t) => ({ name: t.name, description: t.description })),
  };
}
