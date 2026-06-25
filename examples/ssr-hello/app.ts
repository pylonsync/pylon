import {
  buildManifest,
  discoverAppRoutes,
  entity,
  field,
  font,
} from "@pylonsync/sdk";

// Phase 1 smoke test: file-based SSR routing. The discoverer walks
// `app/**/page.tsx` and emits one route per page. Drop a new file
// at `app/about/page.tsx` to add `/about` — no defineRoute() call
// required.
const Marker = entity("Marker", {
  note: field.string(),
});

const manifest = buildManifest({
  name: "ssr-hello",
  version: "0.1.0",
  entities: [Marker],
  queries: [],
  actions: [],
  policies: [],
  routes: await discoverAppRoutes(),
  fonts: [
    font({
      family: "Geist",
      variable: "--font-sans",
      weights: ["400", "600"],
      subsets: ["latin"],
    }),
  ],
});

// Emit canonical manifest JSON to stdout for pylon codegen.
console.log(JSON.stringify(manifest, null, 2));

export default manifest;
