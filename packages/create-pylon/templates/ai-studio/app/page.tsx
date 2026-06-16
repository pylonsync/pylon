import React from "react";
import { type Metadata } from "@pylonsync/react";
import { Studio } from "./studio-client";
import { siteConfig } from "@/lib/site.config";

export const metadata: Metadata = {
  title: siteConfig.seo.title,
  description: siteConfig.seo.description,
  openGraph: { title: siteConfig.seo.title, description: siteConfig.seo.description, type: "website" },
};

// `app/page.tsx` → `/`. A small header over the studio island: a prompt bar +
// kind selector + a live gallery that fills in as generations finish. The layout
// renders the slim top bar.
export default function Home() {
  const { studio } = siteConfig;
  return (
    <div className="min-h-[calc(100vh-3.5rem)] bg-white">
      <div className="mx-auto max-w-5xl px-4 pt-8 text-center sm:pt-12">
        <h1 className="text-balance text-3xl font-semibold tracking-[-0.02em] text-zinc-900 sm:text-4xl">
          {studio.headline}
        </h1>
        <p className="mx-auto mt-3 max-w-xl text-[15px] leading-relaxed text-zinc-500">{studio.subcopy}</p>
      </div>
      <Studio />
    </div>
  );
}
