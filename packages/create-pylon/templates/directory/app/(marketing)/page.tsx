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
  const { intro } = siteConfig;
  return (
    <div className={`${WRAP} pb-20 pt-10 sm:pt-14`}>
      <h1 className="text-balance text-[1.75rem] font-semibold leading-tight tracking-[-0.02em] text-zinc-900 sm:text-[2.25rem]">
        {intro.headline}
      </h1>
      <p className="mt-2 max-w-2xl text-[15px] leading-relaxed text-zinc-600">{intro.description}</p>
      <div id="browse" className="mt-5">
        <DirectoryBrowse />
      </div>
    </div>
  );
}
