import React from "react";
import type { Metadata } from "@pylonsync/react";
import { Catalog } from "../client/Catalog";

export const metadata: Metadata = {
  title: "Pylon Store — 10,000 products, faceted search",
  description:
    "A full e-commerce demo on one Pylon binary: server-rendered pages, faceted full-text search, a sync-backed cart, and order tracking.",
};

// `app/page.tsx` → `/`. The catalog is a client island (live faceted search +
// sync cart), but its filters live in the URL (?category=…&sort=…&q=…) so a
// filtered view is shareable and pre-applies on load. Product cards link to
// the real SSR detail route at /p/<slug>.
export default function Page() {
  return <Catalog />;
}
