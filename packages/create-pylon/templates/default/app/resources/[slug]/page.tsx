import React from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { ContentPage } from "@/components/marketing";
import { RESOURCES, bySlug } from "@/lib/site";

export function generateMetadata({ params }: PageProps): Metadata {
  const page = bySlug(RESOURCES, params.slug);
  if (!page) return { title: "Not found — Acme", robots: "noindex" };
  return { title: `${page.navLabel} — Acme`, description: page.summary };
}

// `/resources/:slug` — docs, guides, changelog, API reference, status. Driven
// by RESOURCES in lib/site.ts. Unknown slugs 404.
export default function ResourcePage({ params, auth, response }: PageProps) {
  const page = bySlug(RESOURCES, params.slug);
  if (!page) {
    response.notFound();
    return null;
  }
  return (
    <ContentPage
      page={page}
      siblings={RESOURCES}
      basePath="/resources"
      ctaHref={auth.user_id ? "/dashboard" : "/signup"}
    />
  );
}
