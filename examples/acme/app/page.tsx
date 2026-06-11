import React from "react";
import { Link, Image } from "@pylonsync/react";
import type { Metadata } from "@pylonsync/react";
import { EarlyAccessForm } from "../client/ContactForm";

export const metadata: Metadata = {
  title: "Acme — Where the work comes together",
  description:
    "Acme keeps your team's projects, docs, and updates in one place — so work stops slipping between tools.",
};

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
      <h1 className="display mx-auto max-w-5xl text-[56px] text-foreground sm:text-[80px] md:text-[96px]">
        Where the work comes together.
      </h1>
      <p className="mx-auto mt-8 max-w-xl text-lg text-muted-foreground">
        Acme keeps your team's projects, docs, and updates in one place — so
        work stops slipping between tools and everyone sees where things stand.
      </p>
      <div className="mt-10 flex justify-center">
        <EarlyAccessForm buttonLabel="Get started free" source="hero" />
      </div>
      <p className="mt-4 text-xs text-muted-foreground">
        Free for your first three projects. No credit card.
      </p>
    </section>
  );
}

function ProductShowcase() {
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

const FEATURES = [
  {
    icon: "✺",
    title: "Projects",
    desc: "Boards, lists, and timelines that stay in sync for everyone the moment anything changes.",
    expanded: true,
  },
  { icon: "✱", title: "Docs in context", expanded: false },
  { icon: "▤", title: "Automations", expanded: false },
  { icon: "⟨/⟩", title: "Search across everything", expanded: false },
];

function FeatureExplorer() {
  return (
    <section className="grid grid-cols-1 gap-12 pb-32 md:grid-cols-[1fr_1.2fr] md:gap-20">
      <div className="md:pt-12">
        <h2 className="display text-[44px] text-foreground sm:text-[56px]">
          Everything,
          <br />
          in one place.
        </h2>
        <ul className="mt-12 divide-y divide-border border-y border-border">
          {FEATURES.map((f, i) => (
            <li key={i} className="py-5">
              <div className="flex items-start gap-3">
                <span className="text-[var(--color-blue)]">{f.icon}</span>
                <div className="flex-1">
                  <h3 className="text-base font-semibold text-foreground">
                    {f.title}
                  </h3>
                  {f.expanded && f.desc && (
                    <p className="mt-2 text-sm leading-relaxed text-muted-foreground">
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
      <h2 className="display text-[44px] text-foreground sm:text-[56px]">
        Loved by teams that ship.
      </h2>
      <div className="mt-12 rounded-3xl bg-card p-8 sm:p-12">
        <div className="grid grid-cols-1 items-center gap-8 md:grid-cols-[1fr_auto]">
          <div>
            <p className="text-sm font-medium text-foreground">Priscilla</p>
            <p className="mt-1 text-xs text-muted-foreground">
              Head of Operations, Northwind
            </p>
            <blockquote className="mt-6 text-2xl leading-snug text-foreground sm:text-3xl">
              "We replaced four tools with Acme and stopped losing track of
              work between them. The whole team finally sees the same thing."
            </blockquote>
            <Link
              href="/contact"
              className="mt-8 inline-flex items-center gap-1 rounded-md bg-background px-3 py-1.5 text-xs font-medium text-muted-foreground shadow-sm transition hover:bg-secondary"
            >
              Talk to us
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
          <span className="h-1.5 w-1.5 rounded-full bg-muted-foreground/30" />
          <span className="h-1.5 w-4 rounded-full bg-foreground" />
          <span className="h-1.5 w-1.5 rounded-full bg-muted-foreground/30" />
          <span className="h-1.5 w-1.5 rounded-full bg-muted-foreground/30" />
        </div>
      </div>
    </section>
  );
}

function SectionWithHighlight() {
  return (
    <section className="pb-16 text-center">
      <h2 className="display mx-auto max-w-3xl text-[44px] text-foreground sm:text-[64px]">
        One source of truth, not <span className="hl">twelve tabs</span>.
      </h2>
    </section>
  );
}

function ProductGrid() {
  return (
    <section className="pb-32">
      <div className="mock overflow-hidden">
        <Image
          src="/hero.jpg"
          alt="Acme overview"
          width={2400}
          height={1500}
          sizes="(max-width: 768px) 100vw, 1100px"
          className="w-full"
        />
      </div>
      <div className="mt-8 grid grid-cols-1 gap-8 md:grid-cols-2">
        <ProductCard
          src="/feat-collab.jpg"
          title="Unified view"
          body="Every project, doc, and update in one place. Jump between them without losing your spot."
        />
        <ProductCard
          src="/feat-speed.jpg"
          title="See what changed"
          body="A live activity feed shows who did what, and when. Catch up in a glance instead of a standup."
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
      <h3 className="mt-6 text-base font-semibold text-foreground">{title}</h3>
      <p className="mt-2 max-w-sm text-sm leading-relaxed text-muted-foreground">
        {body}
      </p>
    </div>
  );
}

function Cta() {
  return (
    <section className="pb-32 text-center">
      <h2 className="display mx-auto max-w-3xl text-[48px] text-foreground sm:text-[72px]">
        Get your team
        <br />
        on the <span className="hl">same page</span>.
      </h2>
      <p className="mx-auto mt-8 max-w-lg text-lg text-muted-foreground">
        Acme sets up in about twelve minutes. Free for your first three
        projects — no credit card.
      </p>
      <div className="mt-10 flex justify-center">
        <EarlyAccessForm buttonLabel="Get started free" source="cta" />
      </div>
    </section>
  );
}
