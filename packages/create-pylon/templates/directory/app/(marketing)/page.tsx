import React from "react";
import { Link, type Metadata } from "@pylonsync/react";
import { WRAP } from "@/components/marketing";
import { DirectoryBrowse } from "./directory-browse";
import { siteConfig } from "@/lib/site.config";

export const metadata: Metadata = {
  title: siteConfig.seo.title,
  description: siteConfig.seo.description,
  openGraph: { title: siteConfig.seo.title, description: siteConfig.seo.description, type: "website" },
};

// `app/page.tsx` → `/`. Server-rendered hero + a client island (#browse) that
// runs a LIVE faceted full-text search over the public Listing table. Copy
// comes from siteConfig; the listings seed on first visit. Doesn't read `auth`,
// so the public page stays cacheable.
export default function LandingPage() {
  const { hero, browse } = siteConfig;
  return (
    <div className="bg-white text-zinc-900">
      {/* HERO */}
      <section className={`${WRAP} pt-16 pb-8 sm:pt-20`}>
        <div className="max-w-2xl">
          <p className="font-mono text-[11px] uppercase tracking-[0.16em] text-brand">{hero.tagline}</p>
          <h1 className="mt-4 text-balance text-[2.5rem] font-semibold leading-[1.04] tracking-[-0.02em] sm:text-[3.25rem]">
            {hero.headline}
          </h1>
          <p className="mt-5 text-[17px] leading-relaxed text-zinc-500">{hero.subcopy}</p>
          <div className="mt-7 flex flex-wrap items-center gap-4">
            <a href="#browse" className="inline-flex items-center rounded-full bg-brand px-5 py-2.5 text-sm font-medium text-white transition-opacity hover:opacity-90">
              Browse the directory
            </a>
            <Link href="/submit" className="text-sm font-medium text-zinc-700 hover:text-zinc-900">
              {hero.ctaLabel} →
            </Link>
          </div>
        </div>
      </section>

      {/* BROWSE — live faceted search */}
      <section id="browse" className={`${WRAP} pb-20 pt-4`}>
        <div className="mb-6">
          <p className="font-mono text-[11px] uppercase tracking-[0.14em] text-brand">{browse.eyebrow}</p>
          <h2 className="mt-3 text-2xl font-semibold tracking-[-0.02em]">{browse.headline}</h2>
        </div>
        <DirectoryBrowse />
      </section>
    </div>
  );
}
