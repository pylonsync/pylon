import type { RawRouteHandler } from "@pylonsync/react";
import { fetchDoc } from "../../../../../lib/agent/tools";

// GET /agent/v1/docs/read?path=concepts/policies — one doc page as markdown.
// operationId: readPylonDoc (see /openapi.json).
//
// `path` is validated against the documentation index before anything is
// fetched. That check is the security boundary: without it this endpoint is an
// open proxy that fetches whatever URL a caller names, from inside our network.
export const GET: RawRouteHandler = async ({ searchParams }) => {
  const path = (searchParams.path ?? "").trim();
  if (!path) {
    return {
      status: 400,
      contentType: "application/json; charset=utf-8",
      headers: { "cache-control": "no-store", "access-control-allow-origin": "*" },
      body: `${JSON.stringify({
        error: {
          code: "MISSING_PATH",
          message:
            "Pass a documentation path as `path`, e.g. /agent/v1/docs/read?path=concepts/policies. List paths at /agent/v1/docs.",
        },
      })}\n`,
    };
  }
  const doc = await fetchDoc(path);
  if (!doc.ok) {
    return {
      status: 404,
      contentType: "application/json; charset=utf-8",
      headers: { "cache-control": "no-store", "access-control-allow-origin": "*" },
      body: `${JSON.stringify({ error: { code: "DOC_NOT_FOUND", message: doc.error } })}\n`,
    };
  }
  const headers: Record<string, string> = {
    "cache-control": "public, max-age=3600",
    "access-control-allow-origin": "*",
    // Point at the source, so a reader can cite the canonical page.
    link: `<${doc.url}>; rel="canonical"`,
  };
  return { contentType: "text/markdown; charset=utf-8", headers, body: doc.markdown };
};
