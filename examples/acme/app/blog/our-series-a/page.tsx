import React from "react";
import { Link } from "@pylonsync/react";

export default function SeriesAPost() {
  return (
    <main className="mx-auto max-w-3xl px-6 pt-16 pb-24 md:pt-24">
      <Link
        href="/blog"
        className="inline-flex items-center gap-1 text-sm text-[var(--color-stone)] transition hover:text-[var(--color-ink)]"
      >
        ← Back to blog
      </Link>
      <div className="mt-8">
        <span className="text-xs font-medium uppercase tracking-wider text-[var(--color-brand-deep)]">
          Company · May 2026
        </span>
        <h1 className="display mt-3 text-4xl text-[var(--color-ink)] sm:text-5xl">
          Acme raises $24M to make the work tools your team won't quit.
        </h1>
        <div className="mt-8 flex items-center gap-3">
          <span className="flex h-10 w-10 items-center justify-center rounded-full bg-[var(--color-accent)] text-xs font-semibold text-white">
            MC
          </span>
          <div>
            <p className="text-sm font-medium text-[var(--color-ink)]">
              Mara Chen
            </p>
            <p className="text-xs text-[var(--color-stone)]">
              Co-founder, CEO · 4 min read
            </p>
          </div>
        </div>
      </div>

      <div className="mt-12 aspect-[16/9] w-full rounded-2xl bg-gradient-to-br from-[var(--color-accent-soft)] to-[var(--color-cream-deep)]" />

      <article className="prose mt-12 max-w-none">
        <p className="text-lg leading-relaxed text-[var(--color-ink-soft)]">
          Today we're sharing that Acme has raised $24M in Series A funding led
          by Index Ventures, with continued participation from our seed
          investors. This is real news to us and we wanted to put it in front
          of you ourselves, in plain English, on the same blog where we ship
          our changelog.
        </p>

        <h2 className="display mt-12 text-2xl text-[var(--color-ink)]">
          What we said we'd do.
        </h2>
        <p className="mt-4 leading-relaxed text-[var(--color-stone)]">
          Eighteen months ago, we showed up to our first investor meetings with
          a working product, eleven customers, and a strong opinion about why
          the way teams work was broken. We promised three things: ship every
          Friday, charge a fair price, never sell out the road map for an
          enterprise deal.
        </p>
        <p className="mt-4 leading-relaxed text-[var(--color-stone)]">
          We've shipped on 64 of the last 68 Fridays. The four exceptions are
          documented in our weekly notes. Pricing is on the pricing page. The
          road map is{" "}
          <Link href="/" className="text-[var(--color-ink)] underline">
            on the site
          </Link>
          .
        </p>

        <h2 className="display mt-12 text-2xl text-[var(--color-ink)]">
          What we're going to do with it.
        </h2>
        <p className="mt-4 leading-relaxed text-[var(--color-stone)]">
          We're hiring twelve people across engineering and design. We're
          deepening automations so more of the routine runs itself — handoffs,
          reminders, status rollups. We're opening a London office because
          half our customers are over there and time zones aren't a feature.
        </p>
        <p className="mt-4 leading-relaxed text-[var(--color-stone)]">
          And we're going to keep being deliberately boring. No new product
          lines we don't believe in. No mystery enterprise tier with secret
          features. No emails from "Acme Customer Success" trying to upsell
          you into something you didn't ask for.
        </p>

        <h2 className="display mt-12 text-2xl text-[var(--color-ink)]">
          Thanks.
        </h2>
        <p className="mt-4 leading-relaxed text-[var(--color-stone)]">
          To the 9,400 teams using Acme — thank you for trusting an early-
          stage product with the surface area of your work day. To the
          twelve people at Acme right now — you're the team I'd hire over
          again twelve times in a row. To you, reading: if any of this
          sounded like the kind of company you'd want to work with, we're at{" "}
          <a
            href="mailto:hello@acme.example"
            className="text-[var(--color-ink)] underline"
          >
            hello@acme.example
          </a>
          .
        </p>
      </article>

      <hr className="mt-16 border-[var(--color-line)]" />
      <div className="mt-12 flex items-center justify-between">
        <Link
          href="/blog"
          className="text-sm text-[var(--color-stone)] transition hover:text-[var(--color-ink)]"
        >
          ← All posts
        </Link>
        <Link
          href="/sign-up"
          className="rounded-full bg-[var(--color-ink)] px-4 py-2 text-sm font-medium text-[var(--color-cream)] transition hover:bg-[var(--color-ink-soft)]"
        >
          Try Acme →
        </Link>
      </div>
    </main>
  );
}
