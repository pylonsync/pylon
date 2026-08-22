import { serializeJsonLd } from "./jsonld";

/**
 * Emit one JSON-LD graph.
 *
 * Rendered in the page body rather than hoisted into `<head>`: JSON-LD is
 * valid anywhere in the document, and a server component rendering into the
 * body needs no coordination with the layout's head.
 *
 * The payload goes in as a text child. `<script>` is a raw-text element, so
 * the serializer — not React — is what keeps a `</script` inside a value from
 * closing the tag early; see `serializeJsonLd`, which escapes every `<`.
 */
export function JsonLd({ graph }: { graph: unknown }) {
  return <script type="application/ld+json">{serializeJsonLd(graph)}</script>;
}
