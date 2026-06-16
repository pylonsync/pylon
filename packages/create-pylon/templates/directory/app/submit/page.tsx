import React from "react";
import { type Metadata } from "@pylonsync/react";
import { WRAP } from "@/components/marketing";
import { SubmitForm } from "../submit-form";
import { siteConfig } from "@/lib/site.config";

export const metadata: Metadata = {
  title: `Submit a tool — ${siteConfig.brand.name}`,
  description: siteConfig.submit.subcopy,
};

// `app/submit/page.tsx` → `/submit`. Server-rendered shell; the form is a client
// island that calls the public submitListing mutation. Submissions land in the
// owner's moderation queue (the deny-all Submission table) — not the public
// directory — until approved.
export default function SubmitPage() {
  const { submit } = siteConfig;
  return (
    <section className={`${WRAP} py-16`}>
      <div className="mx-auto max-w-2xl">
        <p className="font-mono text-[11px] uppercase tracking-[0.16em] text-brand">{submit.eyebrow}</p>
        <h1 className="mt-4 text-balance text-3xl font-semibold tracking-[-0.02em]">{submit.headline}</h1>
        <p className="mt-4 text-[15px] leading-relaxed text-zinc-500">{submit.subcopy}</p>
        <div className="mt-8">
          <SubmitForm />
        </div>
      </div>
    </section>
  );
}
