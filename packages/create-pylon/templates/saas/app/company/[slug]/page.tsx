import React from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { ContentPage } from "@/components/marketing";
import { COMPANY, bySlug } from "@/lib/site";

export function generateMetadata({ params }: PageProps): Metadata {
  const page = bySlug(COMPANY, params.slug);
  if (!page) return { title: "Not found — Acme", robots: "noindex" };
  return { title: `${page.navLabel} — Acme`, description: page.summary };
}

// `/company/:slug` — about, blog, careers, contact, privacy. Driven by COMPANY
// in lib/site.ts. Unknown slugs 404.
export default function CompanyPage({ params, auth, response }: PageProps) {
  const page = bySlug(COMPANY, params.slug);
  if (!page) {
    response.notFound();
    return null;
  }
  return (
    <ContentPage
      page={page}
      siblings={COMPANY}
      basePath="/company"
      ctaHref={auth.user_id ? "/dashboard" : "/signup"}
    />
  );
}
