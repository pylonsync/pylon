import React from "react";
import { Link, Image } from "@pylonsync/react";

export default function HomePage() {
  return (
    <main>
      <Hero />
      <LogoStrip />
      <FeatureGrid />
      <ShowcaseSection />
      <StatsRow />
      <Testimonial />
      <Cta />
    </main>
  );
}

function Hero() {
  return (
    <section className="relative overflow-hidden">
      <div className="mx-auto max-w-6xl px-6 pt-20 pb-24 md:pt-28 md:pb-32">
        <span className="inline-flex items-center gap-2 rounded-full border border-[var(--color-line)] bg-white/60 px-3 py-1 text-xs font-medium text-[var(--color-stone)]">
          <span className="h-1.5 w-1.5 rounded-full bg-[var(--color-brand)]" />
          New · Acme Series A — read the announcement
        </span>
        <h1 className="display mt-8 max-w-3xl text-[44px] text-[var(--color-ink)] sm:text-6xl md:text-[72px]">
          The intelligent operating system for modern teams.
        </h1>
        <p className="mt-6 max-w-xl text-lg leading-relaxed text-[var(--color-stone)]">
          Acme replaces the patchwork of tools your team is stitching together.
          One workspace, one keyboard shortcut, one source of truth — built so
          your best people don't waste a Monday on busywork.
        </p>
        <div className="mt-10 flex flex-wrap items-center gap-4">
          <Link
            href="/sign-up"
            className="rounded-full bg-[var(--color-ink)] px-6 py-3 text-sm font-medium text-[var(--color-cream)] shadow-sm transition hover:bg-[var(--color-ink-soft)]"
          >
            Start free
          </Link>
          <Link
            href="/pricing"
            className="group inline-flex items-center gap-2 rounded-full px-6 py-3 text-sm font-medium text-[var(--color-ink)] transition hover:bg-[var(--color-cream-deep)]"
          >
            See pricing
            <span
              aria-hidden
              className="transition group-hover:translate-x-0.5"
            >
              →
            </span>
          </Link>
        </div>
        <div className="mt-6 flex items-center gap-4 text-xs text-[var(--color-stone)]">
          <span className="inline-flex items-center gap-1.5">
            <CheckIcon />
            No credit card
          </span>
          <span className="inline-flex items-center gap-1.5">
            <CheckIcon />
            14-day trial
          </span>
          <span className="inline-flex items-center gap-1.5">
            <CheckIcon />
            SOC 2 Type II
          </span>
        </div>
      </div>
      <div className="mx-auto max-w-6xl px-6 pb-24">
        <div className="overflow-hidden rounded-2xl border border-[var(--color-line)] bg-white shadow-sm">
          <Image
            src="/product-shot.jpg"
            alt="Acme product screenshot"
            width={2000}
            height={1300}
            priority
            sizes="(max-width: 768px) 100vw, 1200px"
            className="w-full"
          />
        </div>
      </div>
    </section>
  );
}

function LogoStrip() {
  // Acme-style "trusted by" row, but with wordmark placeholders so
  // the template doesn't ship real customer logos.
  const logos = [
    "Northwind",
    "Initech",
    "Hooli",
    "Pied Piper",
    "Stark Ind.",
    "Wayne",
    "Massive Dynamic",
  ];
  return (
    <section className="border-y border-[var(--color-line)] bg-[var(--color-cream-deep)]/50">
      <div className="mx-auto max-w-6xl px-6 py-10">
        <p className="text-center text-xs font-medium uppercase tracking-wider text-[var(--color-stone)]">
          Trusted by teams shipping in production
        </p>
        <ul className="mt-6 flex flex-wrap items-center justify-center gap-x-10 gap-y-4 text-[var(--color-stone)]">
          {logos.map((name) => (
            <li
              key={name}
              className="text-lg font-semibold tracking-tight opacity-70 transition hover:opacity-100"
              style={{ fontFamily: "var(--font-display)" }}
            >
              {name}
            </li>
          ))}
        </ul>
      </div>
    </section>
  );
}

function FeatureGrid() {
  const features = [
    {
      eyebrow: "Built-in AI",
      title: "Drafts that sound like you.",
      body: "Acme learns your team's voice and writes the first 80% of every message, ticket, and doc — leaving the judgment to you.",
      src: "/feat-ai.jpg",
    },
    {
      eyebrow: "Real collaboration",
      title: "Everyone, everywhere, in sync.",
      body: "Live cursors, live presence, conflict-free edits. Your team feels like they're in the same room even when they're on three continents.",
      src: "/feat-collab.jpg",
    },
    {
      eyebrow: "Ridiculous speed",
      title: "Sub-30ms on every click.",
      body: "No spinners, no loading screens, no \"give us a moment\" toasts. You forget you're using software. Your team gets the hour back.",
      src: "/feat-speed.jpg",
    },
  ];
  return (
    <section className="mx-auto max-w-6xl px-6 py-24 md:py-32">
      <h2 className="display max-w-2xl text-4xl text-[var(--color-ink)] sm:text-5xl">
        Replaces the stack you're tired of paying for.
      </h2>
      <p className="mt-6 max-w-xl text-lg text-[var(--color-stone)]">
        One workspace. One subscription. One keyboard shortcut to rule them all.
      </p>
      <div className="mt-16 grid grid-cols-1 gap-8 md:grid-cols-3">
        {features.map((f) => (
          <article
            key={f.title}
            className="overflow-hidden rounded-2xl border border-[var(--color-line)] bg-white transition hover:-translate-y-0.5 hover:shadow-md"
          >
            <Image
              src={f.src}
              alt=""
              width={1200}
              height={800}
              sizes="(max-width: 768px) 100vw, 400px"
              className="aspect-[3/2] w-full object-cover"
            />
            <div className="p-6">
              <span className="text-xs font-medium uppercase tracking-wider text-[var(--color-brand-deep)]">
                {f.eyebrow}
              </span>
              <h3 className="mt-3 text-xl font-semibold tracking-tight text-[var(--color-ink)]">
                {f.title}
              </h3>
              <p className="mt-3 text-sm leading-relaxed text-[var(--color-stone)]">
                {f.body}
              </p>
            </div>
          </article>
        ))}
      </div>
    </section>
  );
}

function ShowcaseSection() {
  return (
    <section className="bg-[var(--color-cream-deep)]/40">
      <div className="mx-auto grid max-w-6xl items-center gap-12 px-6 py-24 md:grid-cols-2 md:py-32">
        <div>
          <span className="text-xs font-medium uppercase tracking-wider text-[var(--color-brand-deep)]">
            Made for builders
          </span>
          <h2 className="display mt-3 text-4xl text-[var(--color-ink)] sm:text-5xl">
            The work is the work. Acme handles the rest.
          </h2>
          <p className="mt-6 text-lg leading-relaxed text-[var(--color-stone)]">
            Every team uses 47 SaaS tools and remembers the URLs for six of
            them. Acme is the surface where the other 41 disappear — automations
            wired in, context shared, decisions logged.
          </p>
          <ul className="mt-8 space-y-4 text-[var(--color-ink-soft)]">
            {[
              "Inline tasks, inline meetings, inline approvals.",
              "Connectors for every tool your team won't quit.",
              "Audit logs the legal team won't roll their eyes at.",
              "Keyboard-first. Mouse-second. Manager-third.",
            ].map((line) => (
              <li key={line} className="flex items-start gap-3 text-sm">
                <CheckIcon />
                <span>{line}</span>
              </li>
            ))}
          </ul>
          <div className="mt-10">
            <Link
              href="/pricing"
              className="group inline-flex items-center gap-2 text-sm font-medium text-[var(--color-ink)] transition"
            >
              See what's in every plan
              <span
                aria-hidden
                className="transition group-hover:translate-x-0.5"
              >
                →
              </span>
            </Link>
          </div>
        </div>
        <div className="relative">
          <div className="overflow-hidden rounded-2xl border border-[var(--color-line)] bg-white shadow-sm">
            <Image
              src="/hero.jpg"
              alt="Acme dashboard"
              width={2400}
              height={1500}
              sizes="(max-width: 768px) 100vw, 600px"
              className="w-full"
            />
          </div>
          <div className="absolute -bottom-6 -left-6 hidden rounded-2xl border border-[var(--color-line)] bg-white p-5 shadow-md md:block">
            <p className="text-xs font-medium uppercase tracking-wider text-[var(--color-stone)]">
              Drafted in 0.6s
            </p>
            <p className="mt-2 max-w-[16ch] text-sm font-medium text-[var(--color-ink)]">
              "Sounds good — let's pencil in Thursday."
            </p>
          </div>
        </div>
      </div>
    </section>
  );
}

function StatsRow() {
  const stats = [
    { value: "47", label: "Tools replaced" },
    { value: "8.4h", label: "Returned to your week" },
    { value: "<30ms", label: "p95 interaction latency" },
    { value: "99.99%", label: "Uptime, last 12 months" },
  ];
  return (
    <section className="mx-auto max-w-6xl px-6 py-20">
      <div className="grid grid-cols-2 gap-y-10 border-y border-[var(--color-line)] py-12 md:grid-cols-4">
        {stats.map((s) => (
          <div key={s.label} className="text-center">
            <div className="display text-4xl text-[var(--color-ink)] md:text-5xl">
              {s.value}
            </div>
            <div className="mt-2 text-xs font-medium uppercase tracking-wider text-[var(--color-stone)]">
              {s.label}
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}

function Testimonial() {
  return (
    <section className="mx-auto max-w-4xl px-6 py-24 text-center md:py-32">
      <blockquote className="display text-3xl leading-tight text-[var(--color-ink)] sm:text-4xl md:text-[40px]">
        "Acme is the only piece of software my team
        <span className="text-[var(--color-brand)]"> doesn't roll its eyes at </span>
        in standup. That's the highest praise I can give it."
      </blockquote>
      <figcaption className="mt-10 flex items-center justify-center gap-4">
        <Image
          src="/avatar-1.jpg"
          alt=""
          width={64}
          height={64}
          className="h-12 w-12 rounded-full object-cover"
        />
        <div className="text-left">
          <div className="text-sm font-medium text-[var(--color-ink)]">
            Mara Chen
          </div>
          <div className="text-xs text-[var(--color-stone)]">
            Head of Operations, Northwind
          </div>
        </div>
      </figcaption>
    </section>
  );
}

function Cta() {
  return (
    <section className="mx-auto max-w-6xl px-6 pb-24">
      <div className="relative overflow-hidden rounded-3xl bg-[var(--color-ink)] px-8 py-16 text-center text-[var(--color-cream)] sm:px-16 sm:py-24">
        <div
          aria-hidden
          className="absolute inset-x-0 -top-32 mx-auto h-72 w-72 rounded-full bg-[var(--color-brand)] opacity-30 blur-[120px]"
        />
        <h2 className="display relative text-4xl sm:text-5xl">
          Your team's best Monday is one signup away.
        </h2>
        <p className="relative mx-auto mt-6 max-w-xl text-base text-[var(--color-stone-soft)]">
          Start free. No credit card. Cancel from inside the app — we won't make
          you find a hidden form.
        </p>
        <div className="relative mt-10 flex flex-wrap items-center justify-center gap-4">
          <Link
            href="/sign-up"
            className="rounded-full bg-[var(--color-cream)] px-6 py-3 text-sm font-medium text-[var(--color-ink)] shadow-sm transition hover:bg-white"
          >
            Start free
          </Link>
          <Link
            href="/pricing"
            className="rounded-full border border-[var(--color-stone)] px-6 py-3 text-sm font-medium text-[var(--color-cream)] transition hover:bg-white/5"
          >
            See pricing
          </Link>
        </div>
      </div>
    </section>
  );
}

function CheckIcon() {
  return (
    <svg
      aria-hidden
      viewBox="0 0 16 16"
      fill="none"
      className="h-3.5 w-3.5 flex-none text-[var(--color-brand)]"
    >
      <circle cx="8" cy="8" r="8" fill="currentColor" opacity="0.15" />
      <path
        d="M4.5 8.5L7 11l5-6"
        stroke="currentColor"
        strokeWidth="1.75"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
