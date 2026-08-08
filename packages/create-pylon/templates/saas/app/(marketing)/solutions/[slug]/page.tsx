import React from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { ContentPage } from "@/components/marketing";
import { SOLUTIONS, bySlug } from "@/lib/site";

export function generateMetadata({ params }: PageProps): Metadata {
  const page = bySlug(SOLUTIONS, params.slug);
  if (!page) return { title: "Not found — Acme", robots: "noindex" };
  return { title: `${page.navLabel} — Acme`, description: page.summary };
}

// `/solutions/:slug` — one template, every solution. Add an entry to SOLUTIONS
// in lib/site.ts and its page exists. Unknown slugs 404.
export default function SolutionPage({ params, auth, response }: PageProps) {
  const page = bySlug(SOLUTIONS, params.slug);
  if (!page) {
    response.notFound();
    return null;
  }
  return (
    <ContentPage
      page={page}
      siblings={SOLUTIONS}
      basePath="/solutions"
      ctaHref={auth.user_id ? "/dashboard" : "/signup"}
    />
  );
}
