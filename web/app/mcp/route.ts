import type { RawRouteHandler, RouteHandler } from "@pylonsync/react";
import {
  INVALID_REQUEST,
  PARSE_ERROR,
  SUPPORTED_PROTOCOL_VERSIONS,
  dispatch,
  isSupportedProtocolVersion,
} from "../../lib/agent/mcp";
import { SITE_URL } from "../../lib/site";

// The Model Context Protocol endpoint, Streamable HTTP transport.
//
// This file is the HTTP shell only: origin validation, protocol-version
// checking, and the status codes the transport specifies. What to ANSWER lives
// in lib/agent/mcp.ts, so the protocol is testable without a socket.
//
// Stateless by construction: no `Mcp-Session-Id` is issued, so none is
// required back and a reconnecting client loses nothing. There is no
// per-session state to keep — every tool is a read.
//
// GET answers 405. The spec says a server MUST either open an SSE stream here
// or return 405; this server has nothing to push, so it says so rather than
// holding a connection open that will never carry a message.

const JSON_CT = "application/json; charset=utf-8";
const NO_STORE = "no-store";

function rpcError(status: number, code: number, message: string) {
  return {
    status,
    contentType: JSON_CT,
    headers: { "cache-control": NO_STORE },
    body: JSON.stringify({ jsonrpc: "2.0", id: null, error: { code, message } }),
  };
}

/**
 * DNS-rebinding defence, required by the transport spec: a page on
 * https://evil.example must not be able to drive this endpoint from inside a
 * victim's browser. A same-origin or absent Origin passes (curl, an SDK, a
 * server); a cross-site browser Origin is refused.
 *
 * This is belt to the framework's braces — Pylon's `trustedOrigins` gate
 * already rejects cross-origin non-GET requests before a handler runs. It
 * stays here so the rule is visible at the endpoint it protects, and survives
 * an edit to the manifest.
 */
function originAllowed(origin: string | undefined): boolean {
  if (!origin) return true;
  try {
    const host = new URL(origin).hostname;
    return (
      host === "www.pylonsync.com" ||
      host === "pylonsync.com" ||
      host === "localhost" ||
      host === "127.0.0.1"
    );
  } catch {
    return false;
  }
}

export const POST: RouteHandler = async ({ headers, body }) => {
  if (!originAllowed(headers.origin)) {
    return rpcError(403, INVALID_REQUEST, "Cross-site origin refused.");
  }

  // A client MUST send `MCP-Protocol-Version` after initialization, and an
  // unsupported value MUST be a 400. A missing header means "the version we
  // negotiated" — for a stateless server, the newest one we speak.
  const requested = headers["mcp-protocol-version"]?.trim();
  if (requested && !isSupportedProtocolVersion(requested)) {
    return {
      status: 400,
      contentType: JSON_CT,
      headers: {
        "cache-control": NO_STORE,
        "mcp-protocol-versions": SUPPORTED_PROTOCOL_VERSIONS.join(", "),
      },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: null,
        error: {
          code: INVALID_REQUEST,
          message: `Unsupported MCP-Protocol-Version "${requested}". This server speaks ${SUPPORTED_PROTOCOL_VERSIONS.join(", ")}.`,
        },
      }),
    };
  }

  let message: unknown;
  try {
    message = JSON.parse(body);
  } catch {
    return rpcError(400, PARSE_ERROR, "Request body is not valid JSON.");
  }

  // A JSON-RPC batch is an array. Batching was removed from the protocol in
  // 2025-06-18, so say that instead of half-answering the first element.
  if (Array.isArray(message)) {
    return rpcError(
      400,
      INVALID_REQUEST,
      "Send one JSON-RPC message per request. Batching was removed from MCP in 2025-06-18.",
    );
  }

  const result = await dispatch(message);
  if (result === null) {
    // A notification or a response: accepted, with nothing to say back.
    return { status: 202, headers: { "cache-control": NO_STORE } };
  }
  // Echo the version this exchange actually settled on. On `initialize` that
  // is the one in the result body — the client sent no header yet, and
  // answering with our newest while the body says otherwise is a contradiction
  // a strict client is right to reject.
  const negotiated =
    (result.result as { protocolVersion?: string } | undefined)?.protocolVersion ??
    requested ??
    SUPPORTED_PROTOCOL_VERSIONS[0];
  return {
    status: 200,
    contentType: JSON_CT,
    headers: { "cache-control": NO_STORE, "mcp-protocol-version": negotiated },
    body: JSON.stringify(result),
  };
};

/** No server-initiated stream here. */
export const GET: RawRouteHandler = async () => ({
  status: 405,
  contentType: JSON_CT,
  headers: { allow: "POST", "cache-control": NO_STORE },
  body: JSON.stringify({
    jsonrpc: "2.0",
    id: null,
    error: {
      code: -32601,
      message: `This MCP server opens no server-initiated stream. POST JSON-RPC to ${SITE_URL}/mcp instead; the manifest is at ${SITE_URL}/mcp.json.`,
    },
  }),
});

/** A stateless server has no session to delete. */
export const DELETE: RouteHandler = async () => ({
  status: 405,
  contentType: JSON_CT,
  headers: { allow: "POST", "cache-control": NO_STORE },
  body: JSON.stringify({
    jsonrpc: "2.0",
    id: null,
    error: {
      code: -32601,
      message: "This MCP server is stateless: it issues no session id, so there is none to end.",
    },
  }),
});
