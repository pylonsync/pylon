import React from "react";
import { type Metadata } from "@pylonsync/react";
import { WRAP } from "@/components/marketing";
import { DirectoryBrowse } from "./directory-browse";
import { siteConfig } from "@/lib/site.config";

export const metadata: Metadata = {
  title: siteConfig.seo.title,
  description: siteConfig.seo.description,
  openGraph: { title: siteConfig.seo.title, description: siteConfig.seo.description, type: "website" },
};

// `app/page.tsx` → `/`. A short server-rendered intro (brand name and one
// line), then the client island (#browse): the search box, the category rail,
// and the list, all driven by a LIVE faceted full-text search over the public
// Listing table. Copy comes from siteConfig; the listings seed on first visit.
// Doesn't read `auth`, so the public page stays cacheable.
export default function LandingPage() {
  const { brand, intro } = siteConfig;
  return (
    <div className={`${WRAP} pb-20 pt-8`}>
      <h1 className="font-display text-[22px] leading-tight text-zinc-900">{brand.name}</h1>
      <p className="mt-1 max-w-2xl text-[14px] leading-relaxed text-zinc-600">{intro.description}</p>
      <div id="browse" className="mt-5">
        <DirectoryBrowse />
      </div>
    </div>
  );
}
