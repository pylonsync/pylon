import React from "react";
import { ContactPageForm } from "../../client/ContactPageForm";

export default function ContactPage() {
  return (
    <main className="mx-auto max-w-3xl px-6 pt-20 pb-24 md:pt-28">
      <span className="text-xs font-medium uppercase tracking-wider text-[var(--color-brand-deep)]">
        Contact
      </span>
      <h1 className="display mt-3 text-4xl text-[var(--color-ink)] sm:text-6xl">
        Let's talk.
      </h1>
      <p className="mt-8 max-w-xl text-lg leading-relaxed text-[var(--color-stone)]">
        Questions about pricing, security, integrations, or just want to see
        the product in action? Send us a note — we read every message and reply
        within one business day.
      </p>

      <div className="mt-12 grid grid-cols-1 gap-12 md:grid-cols-2">
        <ContactPageForm />

        <div className="space-y-8">
          <ContactInfo
            label="Sales &amp; pricing"
            value="hello@acme.example"
            href="mailto:hello@acme.example"
          />
          <ContactInfo
            label="Security disclosures"
            value="security@acme.example"
            href="mailto:security@acme.example"
          />
          <ContactInfo
            label="Headquarters"
            value="Dallas, TX"
          />
          <ContactInfo
            label="Support SLA"
            value="4-hour response on the Team plan, same-day on Starter."
          />
        </div>
      </div>
    </main>
  );
}

function ContactInfo({
  label,
  value,
  href,
}: {
  label: string;
  value: string;
  href?: string;
}) {
  return (
    <div>
      <p className="text-xs font-semibold uppercase tracking-wider text-[var(--color-stone)]">
        {label}
      </p>
      {href ? (
        <a
          href={href}
          className="mt-1 block text-sm text-[var(--color-ink)] underline underline-offset-4 hover:text-[var(--color-brand-deep)]"
        >
          {value}
        </a>
      ) : (
        <p className="mt-1 text-sm text-[var(--color-ink-soft)]">{value}</p>
      )}
    </div>
  );
}
