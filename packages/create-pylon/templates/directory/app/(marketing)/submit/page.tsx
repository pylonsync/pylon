import React from "react";
import { type Metadata } from "@pylonsync/react";
import { WRAP } from "@/components/marketing";
import { SubmitForm } from "../submit-form";
import { siteConfig } from "@/lib/site.config";

export const metadata: Metadata = {
  title: `Submit a tool, ${siteConfig.brand.name}`,
  description: siteConfig.submit.subcopy,
};

// `app/submit/page.tsx` → `/submit`. Server-rendered shell; the form is a client
// island that calls the public submitListing mutation. Submissions land in the
// owner's moderation queue (the deny-all Submission table), not the public
// directory, until approved.
export default function SubmitPage() {
  const { submit } = siteConfig;
  return (
    <div className={`${WRAP} pb-20 pt-8`}>
      <div className="max-w-xl">
        <h1 className="font-display text-[22px] leading-tight text-zinc-900">{submit.headline}</h1>
        <p className="mt-1 text-[14px] leading-relaxed text-zinc-600">{submit.subcopy}</p>
        <div className="mt-6">
          <SubmitForm />
        </div>
      </div>
    </div>
  );
}
