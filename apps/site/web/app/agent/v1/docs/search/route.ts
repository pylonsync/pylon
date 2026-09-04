import type { RawRouteHandler } from "@pylonsync/react";
import { searchDocs } from "../../../../../lib/agent/tools";

// GET /agent/v1/docs/search?q=&limit= — ranked documentation search.
// operationId: searchPylonDocs (see /openapi.json).
export const GET: RawRouteHandler = async ({ searchParams }) => {
  const query = (searchParams.q ?? "").trim();
  if (!query) {
    return {
      status: 400,
      contentType: "application/json; charset=utf-8",
      headers: { "cache-control": "no-store", "access-control-allow-origin": "*" },
      body: `${JSON.stringify({
        error: {
          code: "MISSING_QUERY",
          message: "Pass a search term as `q`, e.g. /agent/v1/docs/search?q=policies.",
        },
      })}\n`,
    };
  }
  const limit = Number.parseInt(searchParams.limit ?? "", 10);
  const results = await searchDocs(query, Number.isFinite(limit) ? limit : 10);
  return {
    contentType: "application/json; charset=utf-8",
    headers: {
      "cache-control": "public, max-age=3600",
      "access-control-allow-origin": "*",
    },
    body: `${JSON.stringify({ query, count: results.length, results }, null, 2)}\n`,
  };
};
