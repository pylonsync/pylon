import type { RawRouteHandler } from "@pylonsync/react";
import { listTemplates } from "../../../../lib/agent/tools";

// GET /agent/v1/templates?q= — starter templates and their scaffold commands.
// operationId: listPylonTemplates (see /openapi.json).
export const GET: RawRouteHandler = async ({ searchParams }) => {
  const query = (searchParams.q ?? "").toLowerCase().trim();
  let templates = listTemplates();
  if (query) {
    const terms = query.split(/\s+/).filter(Boolean);
    templates = templates.filter((t) => {
      const haystack = `${t.template} ${t.name} ${t.blurb} ${t.shows.join(" ")}`.toLowerCase();
      return terms.every((term) => haystack.includes(term));
    });
  }
  return {
    contentType: "application/json; charset=utf-8",
    headers: {
      // Templates change with a release, not with the hour.
      "cache-control": "public, max-age=86400",
      "access-control-allow-origin": "*",
    },
    body: `${JSON.stringify({ count: templates.length, templates }, null, 2)}\n`,
  };
};
