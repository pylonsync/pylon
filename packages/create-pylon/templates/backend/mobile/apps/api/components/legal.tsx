import React from "react";
import { site } from "@/lib/site";

/**
 * The frame both legal pages share: title, effective date, and the notice
 * that this text is a starting point rather than reviewed legal advice.
 *
 * The notice is shown until `lib/site.ts` has your company details, on the
 * same principle as the banner in the site layout: unfinished legal text
 * should say so.
 */
export function LegalPage({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  const unfinished = !site.company || !site.supportEmail || !site.address;
  return (
    <article className="mx-auto max-w-3xl px-6 py-16 sm:py-20">
      <h1 className="text-3xl font-semibold tracking-[-0.02em]">{title}</h1>
      <p className="mt-3 text-[13px] text-ink-muted">
        Effective {formatDate(site.legalEffective)}
        {site.company ? ` · ${site.company}` : ""}
      </p>
      {unfinished ? (
        <p className="mt-8 rounded-xl border border-amber-300 bg-amber-50 px-4 py-3 text-[13px] leading-relaxed text-amber-900">
          Draft. This text describes what the app in this template actually
          does, which makes it a useful starting point, but it has not been
          reviewed by a lawyer and it does not know your business. Fill in
          <code className="mx-1 font-mono">lib/site.ts</code>, read every
          section, and have counsel check it before you submit to a store.
        </p>
      ) : null}
      <div className="prose-legal mt-10">{children}</div>
      <p className="mt-12 border-t border-line pt-6 text-[13px] text-ink-muted">
        Questions about this page?{" "}
        {site.supportEmail ? (
          <a href={`mailto:${site.supportEmail}`}>{site.supportEmail}</a>
        ) : (
          "Add a support address in lib/site.ts."
        )}
      </p>
    </article>
  );
}

/** The legal entity, or a visible placeholder while the config is blank. */
export function Company() {
  return <>{site.company || "[your company]"}</>;
}

function formatDate(iso: string): string {
  // Build from the parts. `new Date("2026-01-01")` parses as UTC midnight and
  // renders as the day before in any negative-UTC timezone.
  const [y, m, d] = iso.split("-").map(Number);
  if (!y || !m || !d) return iso;
  return new Date(y, m - 1, d).toLocaleDateString("en-US", {
    year: "numeric",
    month: "long",
    day: "numeric",
  });
}
