import React from "react";
import { Link, Image } from "@pylonsync/react";

export default function HomePage() {
  return (
    <main className="mx-auto max-w-6xl px-6">
      <Hero />
      <ProductShowcase />
      <FeatureExplorer />
      <Testimonial />
      <SectionWithHighlight />
      <ProductGrid />
      <Cta />
    </main>
  );
}

function Hero() {
  return (
    <section className="pt-24 pb-20 text-center md:pt-32 md:pb-28">
      <h1 className="display mx-auto max-w-5xl text-[56px] text-[var(--color-ink)] sm:text-[80px] md:text-[96px]">
        The intelligent workspace.
      </h1>
      <p className="mx-auto mt-8 max-w-xl text-lg text-[var(--color-muted)]">
        Acme learns how your team works, drafts the busywork, and hands you
        back the day. So you can focus on what matters.
      </p>
      <div className="mt-10">
        <Link
          href="/sign-up"
          className="inline-flex items-center justify-center rounded-lg bg-[var(--color-ink)] px-5 py-2.5 text-sm font-medium text-white shadow-sm transition hover:bg-[var(--color-ink-soft)]"
        >
          Get Started
        </Link>
      </div>
    </section>
  );
}

function ProductShowcase() {
  // Soft gradient background card framing the product mockup.
  return (
    <section className="pb-24">
      <div className="relative overflow-hidden rounded-3xl bg-gradient-to-br from-[#a5c3ff] via-[#dabffb] to-[#ffc09f] p-6 sm:p-10">
        <div className="mock overflow-hidden">
          <Image
            src="/product-shot.jpg"
            alt="Acme product"
            width={2000}
            height={1300}
            priority
            sizes="(max-width: 768px) 100vw, 1100px"
            className="w-full"
          />
        </div>
      </div>
    </section>
  );
}

function FeatureExplorer() {
  // Acme's "Replies drafted in seconds" split: bold title + accordion
  // feature list on the left, large product mockup on the right.
  const features = [
    {
      icon: "✺",
      title: "AI drafts",
      desc: "Acme writes the first 80% of every message in your team's voice. Ready to send in one click.",
      expanded: true,
    },
    { icon: "✱", title: "Inline AI Assistant" },
    { icon: "▤", title: "Scheduling made simple" },
    { icon: "⟨/⟩", title: "Snippets" },
  ];
  return (
    <section className="grid grid-cols-1 gap-12 pb-32 md:grid-cols-[1fr_1.2fr] md:gap-20">
      <div className="md:pt-12">
        <h2 className="display text-[44px] text-[var(--color-ink)] sm:text-[56px]">
          Replies drafted
          <br />
          in seconds.
        </h2>
        <ul className="mt-12 divide-y divide-[var(--color-line)] border-y border-[var(--color-line)]">
          {features.map((f, i) => (
            <li key={i} className="py-5">
              <div className="flex items-start gap-3">
                <span className="text-[var(--color-blue)]">{f.icon}</span>
                <div className="flex-1">
                  <h3 className="text-base font-semibold text-[var(--color-ink)]">
                    {f.title}
                  </h3>
                  {f.expanded && f.desc && (
                    <p className="mt-2 text-sm leading-relaxed text-[var(--color-muted)]">
                      {f.desc}
                    </p>
                  )}
                </div>
              </div>
            </li>
          ))}
        </ul>
      </div>
      <div className="mock overflow-hidden">
        <Image
          src="/feat-ai.jpg"
          alt="AI draft preview"
          width={1200}
          height={800}
          sizes="(max-width: 768px) 100vw, 600px"
          className="w-full"
        />
      </div>
    </section>
  );
}

function Testimonial() {
  return (
    <section className="pb-32">
      <h2 className="display text-[44px] text-[var(--color-ink)] sm:text-[56px]">
        Loved by the busiest people.
      </h2>
      <div className="mt-12 rounded-3xl bg-[var(--color-card)] p-8 sm:p-12">
        <div className="grid grid-cols-1 items-center gap-8 md:grid-cols-[1fr_auto]">
          <div>
            <p className="text-sm font-medium text-[var(--color-ink)]">
              Priscilla
            </p>
            <p className="mt-1 text-xs text-[var(--color-muted)]">
              Investor, Redpoint
            </p>
            <blockquote className="mt-6 text-2xl leading-snug text-[var(--color-ink)] sm:text-3xl">
              "Founders remember who got back to them first. Now I'm always
              that person."
            </blockquote>
            <Link
              href="/"
              className="mt-8 inline-flex items-center gap-1 rounded-md bg-white px-3 py-1.5 text-xs font-medium text-[var(--color-ink-soft)] shadow-sm transition hover:bg-[var(--color-page)]"
            >
              View case study
              <span aria-hidden>›</span>
            </Link>
          </div>
          <Image
            src="/avatar-1.jpg"
            alt=""
            width={400}
            height={400}
            sizes="(max-width: 768px) 80vw, 220px"
            className="aspect-square w-full max-w-[220px] rounded-2xl object-cover"
          />
        </div>
        <div className="mt-8 flex justify-center gap-1.5">
          <span className="h-1.5 w-1.5 rounded-full bg-[var(--color-muted-soft)]/50" />
          <span className="h-1.5 w-4 rounded-full bg-[var(--color-ink)]" />
          <span className="h-1.5 w-1.5 rounded-full bg-[var(--color-muted-soft)]/50" />
          <span className="h-1.5 w-1.5 rounded-full bg-[var(--color-muted-soft)]/50" />
        </div>
      </div>
    </section>
  );
}

function SectionWithHighlight() {
  return (
    <section className="pb-16 text-center">
      <h2 className="display mx-auto max-w-3xl text-[44px] text-[var(--color-ink)] sm:text-[64px]">
        Your writing style, not <span className="hl">GPT slop</span>.
      </h2>
    </section>
  );
}

function ProductGrid() {
  // 2-up product showcase below the inline-highlight title.
  return (
    <section className="pb-32">
      <div className="mock overflow-hidden">
        <Image
          src="/hero.jpg"
          alt="Drafted in your style"
          width={2400}
          height={1500}
          sizes="(max-width: 768px) 100vw, 1100px"
          className="w-full"
        />
      </div>
      <div className="mt-8 grid grid-cols-1 gap-8 md:grid-cols-2">
        <ProductCard
          src="/feat-collab.jpg"
          title="Unified Inbox"
          body="All your accounts in one view. Switch between them with one click."
        />
        <ProductCard
          src="/feat-speed.jpg"
          title="See who's reading"
          body="Know when they opened it, on which device, and how many times. Follow up when they're actually paying attention."
        />
      </div>
    </section>
  );
}

function ProductCard({
  src,
  title,
  body,
}: {
  src: string;
  title: string;
  body: string;
}) {
  return (
    <div>
      <div className="mock overflow-hidden">
        <Image
          src={src}
          alt=""
          width={1200}
          height={800}
          sizes="(max-width: 768px) 100vw, 500px"
          className="w-full"
        />
      </div>
      <h3 className="mt-6 text-base font-semibold text-[var(--color-ink)]">
        {title}
      </h3>
      <p className="mt-2 max-w-sm text-sm leading-relaxed text-[var(--color-muted)]">
        {body}
      </p>
    </div>
  );
}

function Cta() {
  return (
    <section className="pb-32 text-center">
      <h2 className="display mx-auto max-w-3xl text-[48px] text-[var(--color-ink)] sm:text-[72px]">
        Stop typing.
        <br />
        Start <span className="hl">shipping</span>.
      </h2>
      <p className="mx-auto mt-8 max-w-lg text-lg text-[var(--color-muted)]">
        Acme takes 90 seconds to set up. Try it free for fourteen days. No
        credit card.
      </p>
      <div className="mt-10">
        <Link
          href="/sign-up"
          className="inline-flex items-center justify-center rounded-lg bg-[var(--color-ink)] px-5 py-2.5 text-sm font-medium text-white shadow-sm transition hover:bg-[var(--color-ink-soft)]"
        >
          Get Started
        </Link>
      </div>
    </section>
  );
}
