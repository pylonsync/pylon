import type { RawRouteHandler } from "@pylonsync/react";
import { mcpManifest } from "../../lib/agent/mcp";

// GET /mcp.json — the static descriptor for the MCP server at /mcp.
//
// Directories and clients that want to know what a server is before connecting
// read a manifest; the handshake is only reachable if you already decided to
// connect. This says the transport, the URL, the tools, and that no auth is
// required, in one unauthenticated GET.
export const GET: RawRouteHandler = async () => ({
  contentType: "application/json; charset=utf-8",
  headers: {
    "cache-control": "public, max-age=3600",
    // Directories fetch this cross-origin.
    "access-control-allow-origin": "*",
  },
  body: `${JSON.stringify(mcpManifest(), null, 2)}\n`,
});
