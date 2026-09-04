import type { RawRouteHandler } from "@pylonsync/react";
import { openApiDocument } from "../../lib/agent/openapi";

// GET /openapi.json — the OpenAPI 3.1 description of the public agent API.
//
// Pretty-printed on purpose: this document is read by people as often as by
// machines, and the bytes are cheap next to a cache hit.
export const GET: RawRouteHandler = async () => ({
  contentType: "application/json; charset=utf-8",
  headers: {
    "cache-control": "public, max-age=3600",
    // Spec browsers (Swagger UI, Scalar, Stoplight) fetch cross-origin.
    "access-control-allow-origin": "*",
  },
  body: `${JSON.stringify(openApiDocument(), null, 2)}\n`,
});
