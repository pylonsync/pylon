import React from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { ComparePage } from "@/components/marketing";
import { COMPARISONS, bySlug } from "@/lib/site";

export function generateMetadata({ params }: PageProps): Metadata {
  const cmp = bySlug(COMPARISONS, params.slug);
  if (!cmp) return { title: "Not found — Acme", robots: "noindex" };
  return { title: `${cmp.title} — Acme`, description: cmp.summary };
}

// `/compare/:slug` — one template per comparison. Driven by COMPARISONS in
// lib/site.ts (generic, made-up competitors). Unknown slugs 404.
export default function CompareSlugPage({ params, auth, response }: PageProps) {
  const cmp = bySlug(COMPARISONS, params.slug);
  if (!cmp) {
    response.notFound();
    return null;
  }
  return (
    <ComparePage
      cmp={cmp}
      all={COMPARISONS}
      ctaHref={auth.user_id ? "/dashboard" : "/signup"}
    />
  );
}
