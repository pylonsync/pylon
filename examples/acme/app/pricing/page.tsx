import React from "react";
import { Link } from "@pylonsync/react";

interface PageProps {
  url: string;
}

const TIERS = [
  {
    name: "Starter",
    price: 0,
    period: "Forever",
    blurb: "For solo founders and small teams trying Acme on for size.",
    features: [
      "Up to 3 seats",
      "Unlimited projects",
      "Docs, boards & timelines",
      "Email support",
    ],
    cta: { label: "Start free", href: "/sign-up" },
    accent: false,
  },
  {
    name: "Team",
    price: 18,
    period: "per seat / month",
    blurb: "When the team starts pulling in different directions.",
    features: [
      "Unlimited seats",
      "Unlimited automations",
      "SSO, audit log, RBAC",
      "Priority support, 4-hour SLA",
      "All integrations",
    ],
    cta: { label: "Start free trial", href: "/sign-up?plan=team" },
    accent: true,
  },
  {
    name: "Enterprise",
    price: null,
    period: "Custom",
    blurb: "When procurement wants to talk and you need a real DPA.",
    features: [
      "SAML, SCIM, custom roles",
      "Dedicated SLA + named CSM",
      "Custom data residency (US / EU)",
      "Annual + invoice billing",
      "Procurement support",
    ],
    cta: { label: "Talk to us", href: "/contact" },
    accent: false,
  },
];

export default function PricingPage({ url }: PageProps) {
  return (
    <main>
      <section className="mx-auto max-w-6xl px-6 pt-20 pb-16 md:pt-28">
        <div className="mx-auto max-w-2xl text-center">
          <span className="text-xs font-medium uppercase tracking-wider text-[var(--color-brand-deep)]">
            Pricing
          </span>
          <h1 className="display mt-3 text-4xl text-[var(--color-ink)] sm:text-6xl">
            Simple, honest, no per-feature surprises.
          </h1>
          <p className="mt-6 text-lg text-[var(--color-stone)]">
            One price, every feature. Pay only for the seats you have, with the
            kind of "this is all the bill, full stop" we wish the rest of the
            SaaS industry would adopt.
          </p>
        </div>

        <div className="mt-16 grid grid-cols-1 gap-6 md:grid-cols-3">
          {TIERS.map((tier) => (
            <article
              key={tier.name}
              className={
                "relative flex flex-col rounded-3xl border p-8 " +
                (tier.accent
                  ? "border-[var(--color-ink)] bg-[var(--color-ink)] text-[var(--color-cream)]"
                  : "border-[var(--color-line)] bg-white text-[var(--color-ink)]")
              }
            >
              {tier.accent && (
                <span className="absolute -top-3 right-6 rounded-full bg-[var(--color-brand)] px-3 py-1 text-xs font-medium uppercase tracking-wider text-[var(--color-cream)]">
                  Most teams
                </span>
              )}
              <div className="flex items-baseline justify-between">
                <h2 className="text-lg font-semibold tracking-tight">
                  {tier.name}
                </h2>
              </div>
              <p
                className={
                  "mt-2 text-sm " +
                  (tier.accent
                    ? "text-[var(--color-stone-soft)]"
                    : "text-[var(--color-stone)]")
                }
              >
                {tier.blurb}
              </p>
              <div className="mt-6 flex items-baseline gap-2">
                <span className="display text-5xl">
                  {tier.price === null ? "Let's talk" : `$${tier.price}`}
                </span>
              </div>
              <div
                className={
                  "mt-1 text-xs uppercase tracking-wider " +
                  (tier.accent
                    ? "text-[var(--color-stone-soft)]"
                    : "text-[var(--color-stone)]")
                }
              >
                {tier.period}
              </div>
              <ul className="mt-8 space-y-3 text-sm">
                {tier.features.map((f) => (
                  <li key={f} className="flex items-start gap-3">
                    <Check accent={tier.accent} />
                    <span>{f}</span>
                  </li>
                ))}
              </ul>
              <Link
                href={tier.cta.href}
                className={
                  "mt-10 inline-flex w-full items-center justify-center rounded-full px-4 py-2.5 text-sm font-medium transition " +
                  (tier.accent
                    ? "bg-[var(--color-cream)] text-[var(--color-ink)] hover:bg-white"
                    : "border border-[var(--color-line)] text-[var(--color-ink)] hover:bg-[var(--color-cream-deep)]")
                }
              >
                {tier.cta.label}
              </Link>
            </article>
          ))}
        </div>
      </section>

      <section className="mx-auto max-w-4xl px-6 py-24">
        <h2 className="display text-3xl text-[var(--color-ink)] sm:text-4xl">
          Frequently asked.
        </h2>
        <dl className="mt-12 divide-y divide-[var(--color-line)] border-y border-[var(--color-line)]">
          {[
            {
              q: "Do you offer non-profit / educational discounts?",
              a: "Yes — 50% off the Team tier for registered non-profits and education. Email hello@acme.example.",
            },
            {
              q: "Is there an annual plan?",
              a: "Annual billing is 20% off and comes with invoice + procurement support out of the box. Switch from inside Settings.",
            },
            {
              q: "Can I self-host Acme?",
              a: "Yes — every paid plan includes the self-hosted Docker image at no extra cost. Bring your own database, bring your own object store.",
            },
            {
              q: "What happens if I cancel?",
              a: "You keep read-only access to your data forever. Cancel from inside the app — no calls, no forms, no retention specialist.",
            },
          ].map((row) => (
            <div key={row.q} className="grid gap-2 py-6 md:grid-cols-3">
              <dt className="text-sm font-medium text-[var(--color-ink)]">
                {row.q}
              </dt>
              <dd className="text-sm leading-relaxed text-[var(--color-stone)] md:col-span-2">
                {row.a}
              </dd>
            </div>
          ))}
        </dl>
        <p className="mt-10 text-sm text-[var(--color-stone)]">
          You're on <code className="text-[var(--color-ink)]">{url}</code> —
          curious about something else?{" "}
          <Link
            href="/contact"
            className="text-[var(--color-ink)] underline underline-offset-4 hover:text-[var(--color-brand-deep)]"
          >
            Just ask.
          </Link>
        </p>
      </section>
    </main>
  );
}

function Check({ accent }: { accent: boolean }) {
  return (
    <svg
      aria-hidden
      viewBox="0 0 16 16"
      fill="none"
      className={
        "mt-0.5 h-4 w-4 flex-none " +
        (accent ? "text-[var(--color-brand)]" : "text-[var(--color-ink)]")
      }
    >
      <path
        d="M3 8.5L6.5 12L13 5"
        stroke="currentColor"
        strokeWidth="1.75"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
