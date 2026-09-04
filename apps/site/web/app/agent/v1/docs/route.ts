import type { RawRouteHandler } from "@pylonsync/react";
import { listDocs } from "../../../../lib/agent/tools";

// GET /agent/v1/docs — the whole documentation index.
// operationId: listPylonDocs (see /openapi.json).
export const GET: RawRouteHandler = async () => {
  const results = await listDocs();
  return {
    contentType: "application/json; charset=utf-8",
    headers: {
      // The index is derived from the docs site's llms.txt, which changes on a
      // docs deploy. An hour is short enough to notice and long enough to keep
      // this off the docs site's back.
      "cache-control": "public, max-age=3600",
      "access-control-allow-origin": "*",
    },
    body: `${JSON.stringify({ count: results.length, results }, null, 2)}\n`,
  };
};
