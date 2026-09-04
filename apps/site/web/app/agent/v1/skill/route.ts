import type { RawRouteHandler } from "@pylonsync/react";
import { readSkill } from "../../../../lib/agent/tools";

// GET /agent/v1/skill — the Pylon authoring guide, as markdown.
// operationId: getPylonSkill (see /openapi.json).
//
// The same bytes as /pylon-skill.md. Both exist on purpose: the static path is
// the one printed on the site and in the README, this one is the versioned API
// surface the OpenAPI document describes.
export const GET: RawRouteHandler = async () => {
  const skill = await readSkill();
  return {
    contentType: "text/markdown; charset=utf-8",
    headers: {
      "cache-control": "public, max-age=3600",
      "access-control-allow-origin": "*",
      link: `<${skill.url}>; rel="canonical"`,
    },
    body: skill.markdown,
  };
};
