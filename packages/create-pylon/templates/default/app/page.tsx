import React from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import {
  WRAP,
  Badge,
  Divider,
  Eyebrow,
  SectionHead,
  FeatureGrid,
  PrimaryButton,
  GhostLink,
  Shot,
  Portrait,
  Terminal,
} from "@/components/marketing";
import { PRODUCTS, productBySlug } from "@/lib/products";

// SEO metadata. Exported `metadata` is rendered into <head> on the server, so
// this marketing page is fully indexable — view source and the copy is in the
// HTML. Swap "Acme" for your product throughout.
export const metadata: Metadata = {
  title: "Acme — the workspace where work gets done",
  description:
    "Acme brings your projects, your people, and your updates into one fast, real-time workspace. Plan together, ship together, keep everyone in the loop.",
};

// The three products the homepage features inline. Each links to its own
// /products/[slug] page for the full story. (Defined once in lib/products.ts.)
const projects = productBySlug("projects")!;
const tasks = productBySlug("tasks")!;
const docs = productBySlug("docs")!;
const automations = productBySlug("automations")!;

// `app/page.tsx` → `/`. A server-rendered marketing landing page. It reads
// `auth` (resolved from the session cookie during the render) so the call to
// action is right on the first byte — "Get started" for visitors, "Open
// dashboard" once you're signed in. No client fetch, no flash.
export default function LandingPage({ auth }: PageProps) {
  const signedIn = Boolean(auth.user_id);
  const primaryHref = signedIn ? "/dashboard" : "/signup";
  const primaryLabel = signedIn ? "Open dashboard" : "Get started";

  return (
    <div className="bg-white text-zinc-900">
      {/* ============================ HERO ============================ */}
      <section className={`${WRAP} pt-20 pb-16 sm:pt-28`}>
        <Badge>Acme for teams is here →</Badge>
        <h1 className="mt-6 max-w-2xl text-balance text-[2.75rem] font-semibold leading-[1.05] tracking-[-0.02em] sm:text-[3.5rem]">
          The workspace where work gets done.
        </h1>
        <p className="mt-6 max-w-xl text-[17px] leading-relaxed text-zinc-500">
          Acme brings your projects, your people, and your updates into one
          fast, real-time workspace. Plan together, ship together, and keep
          everyone in the loop without the busywork.
        </p>
        <div className="mt-8 flex flex-wrap items-center gap-4">
          <PrimaryButton href={primaryHref}>{primaryLabel}</PrimaryButton>
          <GhostLink href="/#product">Take the tour →</GhostLink>
        </div>

        <div className="mt-16">
          <Shot url="acme.app/dashboard" label="Dashboard preview" />
        </div>
      </section>

      {/* ========================= LOGO CLOUD ========================= */}
      <section className={`${WRAP} pb-16`}>
        <p className="font-mono text-[11px] uppercase tracking-[0.14em] text-zinc-400">
          Powering fast-moving teams
        </p>
        <div className="mt-6 flex flex-wrap items-center gap-x-10 gap-y-4">
          {["Northwind", "Globex", "Initech", "Umbrella", "Soylent", "Hooli"].map(
            (name) => (
              <div
                key={name}
                className="flex items-center gap-2 text-zinc-400"
                title={name}
              >
                <span className="size-5 rounded bg-zinc-200" />
                <span className="text-[13px] font-semibold uppercase tracking-wide">
                  {name}
                </span>
              </div>
            ),
          )}
        </div>
      </section>

      {/* ===================== OUTCOMES (2-col) ====================== */}
      <Divider />
      <section className={`${WRAP} py-20`}>
        <div className="grid gap-12 lg:grid-cols-[0.9fr_1.1fr]">
          <div>
            <Eyebrow>Why Acme</Eyebrow>
            <h2 className="mt-4 text-balance text-3xl font-semibold leading-[1.1] tracking-[-0.02em] sm:text-[2.5rem]">
              Everything your team needs to stay in motion.
            </h2>
            <p className="mt-5 max-w-md text-[15px] leading-relaxed text-zinc-500">
              Most teams lose work somewhere between the kickoff and the ship
              date. Acme keeps the whole path in one place, so every idea has a
              clear route from planned, to in progress, to done.
            </p>
          </div>
          <FeatureGrid columns={2} items={OUTCOMES} />
        </div>
      </section>

      {/* ======================= TESTIMONIAL ========================= */}
      <Divider />
      <section className={`${WRAP} py-20`}>
        <div className="grid items-center gap-12 lg:grid-cols-[0.8fr_1.2fr]">
          <Portrait name="Maya Chen" />
          <figure>
            <div className="font-serif text-4xl leading-none text-brand">
              &ldquo;
            </div>
            <blockquote className="mt-4 text-balance text-2xl font-medium leading-[1.35] tracking-[-0.01em] sm:text-[1.75rem]">
              Our whole team finally works in one place. Acme made it easy to see
              what is happening, decide what is next, and keep everyone moving in
              the same direction.
            </blockquote>
            <figcaption className="mt-8 border-t border-zinc-200/70 pt-6">
              <div className="text-sm font-semibold">Maya Chen</div>
              <div className="text-sm text-zinc-500">
                Head of Product, Northwind
              </div>
            </figcaption>
          </figure>
        </div>
      </section>

      {/* =================== FEATURE: PROJECTS ======================= */}
      <Divider />
      <ProductSection id="product" product={projects} primaryHref={primaryHref} />

      {/* ===================== ENTRY POINTS ========================= */}
      <Divider />
      <section className={`${WRAP} py-20`}>
        <SectionHead
          eyebrow="Anywhere"
          title="Meet your team where they already are."
          body="Web, desktop, and a typed API all share the same workspace underneath, so nothing lives in two places."
        />
        <div className="mt-12 grid gap-5 sm:grid-cols-2">
          {ENTRY_POINTS.map((c, i) => (
            <div
              key={c.title}
              className="rounded-2xl border border-zinc-200 bg-paper p-7"
            >
              <div className="flex items-start justify-between">
                <span className="flex size-9 items-center justify-center rounded-lg bg-brand-soft text-brand">
                  {c.icon}
                </span>
                <span className="font-mono text-[11px] text-zinc-300">
                  0{i + 1}
                </span>
              </div>
              <h3 className="mt-5 text-base font-semibold">{c.title}</h3>
              <p className="mt-2 text-[14px] leading-relaxed text-zinc-500">
                {c.body}
              </p>
            </div>
          ))}
        </div>
      </section>

      {/* ====================== FEATURE: TASKS ===================== */}
      <Divider />
      <ProductSection product={tasks} primaryHref={primaryHref} />

      {/* ====================== FEATURE: DOCS ====================== */}
      <Divider />
      <ProductSection product={docs} primaryHref={primaryHref} />

      {/* ============== ENGAGEMENT (prose + numbered) ============== */}
      <Divider />
      <section className={`${WRAP} py-20`}>
        <Eyebrow>Momentum</Eyebrow>
        <h2 className="mt-4 max-w-2xl text-balance text-3xl font-semibold leading-[1.1] tracking-[-0.02em] sm:text-[2.5rem]">
          Most tools go quiet after the kickoff.
        </h2>
        <div className="mt-12 grid gap-12 lg:grid-cols-2">
          <div className="space-y-5 text-[15px] leading-relaxed text-zinc-500">
            <p>
              Someone shares an idea, it lands in a list, and that is the last
              anyone hears of it. Acme treats every idea as the start of a loop,
              not the end of one.
            </p>
            <p>
              When the status moves to planned, the people who care hear about
              it. When it ships, they hear about it first. And every week a
              digest pulls them back with the work worth weighing in on.
            </p>
            <p>
              None of it is something you configure. Turn Acme on, and the loop
              starts running the same day.
            </p>
          </div>
          <ol className="space-y-7">
            {ENGAGEMENT.map((e, i) => (
              <li key={e.title} className="flex gap-4">
                <span className="mt-0.5 font-mono text-[11px] text-zinc-300">
                  0{i + 1}
                </span>
                <div>
                  <span className="text-[15px] font-medium text-brand">
                    {e.title}.
                  </span>{" "}
                  <span className="text-[15px] leading-relaxed text-zinc-500">
                    {e.body}
                  </span>
                </div>
              </li>
            ))}
          </ol>
        </div>
      </section>

      {/* ======================= AUTOMATIONS ======================= */}
      <Divider />
      <section className={`${WRAP} py-20`}>
        <SectionHead
          eyebrow={automations.eyebrow}
          arrow
          title={automations.headline}
          body={automations.summary}
        />
        <FeatureGrid className="mt-14" items={automations.features} />
        <div className="mt-8">
          <GhostLink href="/products/automations">
            Explore Automations →
          </GhostLink>
        </div>
        <div className="mt-12">
          <Terminal />
        </div>
      </section>

      {/* =================== TESTIMONIAL CARDS ===================== */}
      <Divider />
      <section id="customers" className={`${WRAP} py-20`}>
        <SectionHead
          eyebrow="Customers"
          title="Teams that ship with Acme."
          body="A few words from the people who run their work in Acme every day."
        />
        <div className="mt-12 grid gap-5 sm:grid-cols-3">
          {QUOTES.map((q) => (
            <figure
              key={q.name + q.role}
              className="flex flex-col rounded-2xl border border-zinc-200 bg-paper p-6"
            >
              <blockquote className="text-[14px] leading-relaxed text-zinc-600">
                &ldquo;{q.quote}&rdquo;
              </blockquote>
              <figcaption className="mt-6 flex items-center gap-3">
                <span className="flex size-8 items-center justify-center rounded-full bg-zinc-200 text-[11px] font-semibold text-zinc-500">
                  {initials(q.name)}
                </span>
                <div className="leading-tight">
                  <div className="text-[13px] font-semibold">{q.name}</div>
                  <div className="text-[12px] text-zinc-500">{q.role}</div>
                </div>
              </figcaption>
            </figure>
          ))}
        </div>
      </section>

      {/* ===================== GETTING STARTED ==================== */}
      <Divider />
      <section className={`${WRAP} py-20`}>
        <Eyebrow>Get started</Eyebrow>
        <h2 className="mt-4 max-w-xl text-balance text-3xl font-semibold leading-[1.1] tracking-[-0.02em] sm:text-[2.5rem]">
          Up and running in 60 seconds.
        </h2>
        <p className="mt-5 max-w-md text-[15px] leading-relaxed text-zinc-500">
          No credit card, no sales call, no setup wizard. An email and a
          workspace name are all it takes to start.
        </p>
        <div className="mt-8">
          <PrimaryButton href={primaryHref}>{primaryLabel}</PrimaryButton>
        </div>
        <ol className="mt-14 max-w-xl space-y-8">
          {STEPS.map((s, i) => (
            <li key={s.title} className="flex gap-5">
              <span className="flex size-7 shrink-0 items-center justify-center rounded-full bg-brand-soft font-mono text-[11px] text-brand">
                0{i + 1}
              </span>
              <div>
                <h3 className="text-[15px] font-semibold">{s.title}</h3>
                <p className="mt-1.5 text-[14px] leading-relaxed text-zinc-500">
                  {s.body}
                </p>
              </div>
            </li>
          ))}
        </ol>
      </section>

      {/* ========================= PRICING ======================== */}
      <Divider />
      <section id="pricing" className={`${WRAP} py-20`}>
        <Eyebrow>Pricing</Eyebrow>
        <h2 className="mt-4 max-w-xl text-balance text-3xl font-semibold leading-[1.1] tracking-[-0.02em] sm:text-[2.5rem]">
          Simple pricing. Every plan.
        </h2>
        <p className="mt-5 max-w-md text-[15px] leading-relaxed text-zinc-500">
          Start free and upgrade when your team grows. No per-seat surprises, no
          annual lock-in.
        </p>
        <div className="mt-12 grid gap-5 lg:grid-cols-3">
          {PLANS.map((p) => (
            <div
              key={p.name}
              className={`flex flex-col rounded-2xl border p-7 ${
                p.featured
                  ? "border-zinc-900 bg-white shadow-[0_24px_60px_-30px_rgba(0,0,0,0.3)]"
                  : "border-zinc-200 bg-paper"
              }`}
            >
              <div className="flex items-center justify-between">
                <h3 className="text-base font-semibold">{p.name}</h3>
                {p.featured && (
                  <span className="font-mono text-[10px] uppercase tracking-[0.12em] text-brand">
                    Most popular
                  </span>
                )}
              </div>
              <p className="mt-1 text-[13px] text-zinc-500">{p.tagline}</p>
              <div className="mt-5 flex items-baseline gap-1">
                <span className="text-4xl font-semibold tracking-tight">
                  {p.price}
                </span>
                <span className="text-[13px] text-zinc-500">{p.unit}</span>
              </div>
              <ul className="mt-6 flex-1 space-y-3 text-[14px] text-zinc-600">
                {p.features.map((f) => (
                  <li key={f} className="flex gap-2.5">
                    <span className="mt-[3px] text-brand">✓</span>
                    {f}
                  </li>
                ))}
              </ul>
              <div className="mt-7">
                {p.featured ? (
                  <PrimaryButton
                    href={primaryHref}
                    className="w-full justify-center"
                  >
                    {p.cta}
                  </PrimaryButton>
                ) : (
                  <Link
                    href={primaryHref}
                    className="inline-flex w-full items-center justify-center rounded-full border border-zinc-300 px-5 py-2.5 text-sm font-medium text-zinc-900 transition-colors hover:border-zinc-400 hover:bg-zinc-50"
                  >
                    {p.cta}
                  </Link>
                )}
              </div>
            </div>
          ))}
        </div>
      </section>

      {/* ====================== THE TEAM (2-col) ================== */}
      <Divider />
      <section className={`${WRAP} py-20`}>
        <div className="grid gap-12 lg:grid-cols-[0.9fr_1.1fr]">
          <div>
            <Eyebrow>The team</Eyebrow>
            <h2 className="mt-4 text-balance text-3xl font-semibold leading-[1.1] tracking-[-0.02em] sm:text-[2.5rem]">
              Built by people who use it every day.
            </h2>
            <p className="mt-5 max-w-md text-[15px] leading-relaxed text-zinc-500">
              Acme is built by a small team that got tired of clunky, overpriced
              tools. Every feature earns its place by being something worth using
              every day.
            </p>
          </div>
          <FeatureGrid columns={2} items={TEAM} />
        </div>
      </section>

      {/* ======================= FINAL CTA ======================== */}
      <Divider />
      <section className={`${WRAP} py-24`}>
        <Eyebrow>Start building</Eyebrow>
        <h2 className="mt-4 max-w-xl text-balance text-[2.5rem] font-semibold leading-[1.05] tracking-[-0.02em] sm:text-[3rem]">
          Stop losing momentum to busywork.
        </h2>
        <p className="mt-6 max-w-lg text-[16px] leading-relaxed text-zinc-500">
          When your team can see the work, decide what is next, and ship in one
          place, momentum takes care of itself. This is what{" "}
          <span className="rounded bg-brand-soft px-1.5 py-0.5 font-medium text-brand">
            working in flow
          </span>{" "}
          looks like.
        </p>
        <div className="mt-8">
          <PrimaryButton href={primaryHref}>
            Start building with Acme
          </PrimaryButton>
        </div>
        <p className="mt-4 text-[12px] text-zinc-400">
          Free to start · No credit card · Cancel anytime
        </p>
      </section>

      {/* ========================== FAQ =========================== */}
      <Divider />
      <section id="faq" className={`${WRAP} py-20`}>
        <Eyebrow>Questions</Eyebrow>
        <h2 className="mt-4 text-balance text-3xl font-semibold leading-[1.1] tracking-[-0.02em] sm:text-[2.5rem]">
          Frequently asked.
        </h2>
        <div className="mt-10 divide-y divide-zinc-200/70 border-y border-zinc-200/70">
          {FAQ.map((f) => (
            <details key={f.q} className="group py-5">
              <summary className="flex cursor-pointer items-center justify-between text-[15px] font-medium text-zinc-900 marker:hidden [&::-webkit-details-marker]:hidden">
                {f.q}
                <span className="text-brand transition-transform group-open:rotate-45">
                  +
                </span>
              </summary>
              <p className="mt-3 max-w-2xl text-[14px] leading-relaxed text-zinc-500">
                {f.a}
              </p>
            </details>
          ))}
        </div>
      </section>
    </div>
  );
}

// A homepage feature section for one product: eyebrow + headline + grid +
// "Explore →" link to its /products/[slug] page + a product mockup.
function ProductSection({
  product,
  primaryHref,
  id,
}: {
  product: (typeof PRODUCTS)[number];
  primaryHref: string;
  id?: string;
}) {
  return (
    <section id={id} className={`${WRAP} py-20`}>
      <SectionHead
        eyebrow={product.eyebrow}
        arrow
        title={product.headline}
        body={product.summary}
      />
      <FeatureGrid className="mt-14" items={product.features.slice(0, 6)} />
      <div className="mt-8">
        <GhostLink href={`/products/${product.slug}`}>
          Explore {product.title} →
        </GhostLink>
      </div>
      <div className="mt-12">
        <Shot url={product.mockupUrl} label={product.mockupLabel} />
      </div>
    </section>
  );
}

// Initials for the testimonial avatars, so the cards look finished without a
// real photo. Drop in an <img> when you have one.
function initials(name: string) {
  return name
    .split(/\s+/)
    .map((w) => w[0])
    .join("")
    .slice(0, 2)
    .toUpperCase();
}

/* ---------------------------------------------------------------- */
/* Homepage-specific content — fictional demo copy; swap for yours.  */
/* ---------------------------------------------------------------- */

const OUTCOMES = [
  {
    title: "Work in one place",
    body: "Plans, tasks, and docs live together, so nothing gets lost between tools.",
  },
  {
    title: "Clear priorities",
    body: "Everyone can see what is planned, in progress, and done — and why.",
  },
  {
    title: "Real-time by default",
    body: "Every change syncs instantly, so the workspace is the same on every screen.",
  },
  {
    title: "Less busywork",
    body: "Automations handle the routine, so your team focuses on the work that matters.",
  },
];

const ENTRY_POINTS = [
  {
    icon: "◇",
    title: "Cloud workspace",
    body: "Your whole team works in the browser. Nothing to install, always up to date, and secure by default.",
  },
  {
    icon: "◆",
    title: "Open API",
    body: "A typed API and webhooks for everything in Acme, so you can wire it into the rest of your stack.",
  },
];

const ENGAGEMENT = [
  {
    title: "The loop",
    body: "Submission, acknowledgment, progress, launch. Four moments every person gets to feel heard.",
  },
  {
    title: "Instant alerts",
    body: "The moment someone votes or comments on their idea, social proof brings them back.",
  },
  {
    title: "Launch alerts",
    body: "The note that tells a person the thing they asked for just shipped. Nothing builds loyalty faster.",
  },
  {
    title: "Weekly digest",
    body: "The top ideas across your boards, delivered every week, with a one-click way to weigh in.",
  },
];

const QUOTES = [
  {
    quote:
      "We finally have one source of truth for the work. The whole team can see what is happening without a status meeting.",
    name: "Daniel Reyes",
    role: "Founder, Globex",
  },
  {
    quote:
      "Acme noticeably improved how we plan. Instead of piecing together five tools, we have one hub for everything.",
    name: "Priya Sharma",
    role: "Founder, OpenLane",
  },
  {
    quote:
      "Ever since we added Acme, people actually feel heard. It has helped us build a loyal community of users.",
    name: "Marcus Bell",
    role: "Cofounder, Initech",
  },
];

const STEPS = [
  {
    title: "Create a workspace",
    body: "Sign up with just an email. Pick a name. That is the entire form.",
  },
  {
    title: "Invite your team",
    body: "Add teammates, share the board URL, or link it from your site. Work starts flowing in.",
  },
  {
    title: "Watch the loop start",
    body: "People add ideas, votes pile up, you ship, and everyone hears about it and comes back with more.",
  },
];

const PLANS = [
  {
    name: "Free",
    tagline: "For getting started.",
    price: "$0",
    unit: "forever",
    cta: "Get started",
    featured: false,
    features: [
      "Up to 3 projects",
      "Unlimited members",
      "Real-time board",
      "Community support",
    ],
  },
  {
    name: "Team",
    tagline: "For small teams.",
    price: "$29",
    unit: "/ month",
    cta: "Start free trial",
    featured: true,
    features: [
      "Unlimited projects",
      "Roadmap and updates",
      "Private boards",
      "Priority support",
    ],
  },
  {
    name: "Business",
    tagline: "For growing teams.",
    price: "$59",
    unit: "/ month",
    cta: "Start free trial",
    featured: false,
    features: [
      "Everything in Team",
      "SSO and audit log",
      "Custom domain",
      "Onboarding help",
    ],
  },
];

const TEAM = [
  {
    title: "A small team",
    body: "No committees. Every feature ships because someone decided it was worth building.",
  },
  {
    title: "Design first",
    body: "Beautiful software makes people want to use it. Every screen is built with that in mind.",
  },
  {
    title: "Ships every week",
    body: "Updates go out weekly, often based on work posted directly to our own board.",
  },
  {
    title: "Customer funded",
    body: "We optimize for your renewal, not an exit. If Acme works for you, it works for us.",
  },
];

const FAQ = [
  {
    q: "Is Acme a fit for my team?",
    a: "Acme works for any team that plans and ships work together — product, design, engineering, or ops.",
  },
  {
    q: "What about migrating from another tool?",
    a: "Import your existing projects and pick up where you left off.",
  },
  {
    q: "Is there an API or a way to script this?",
    a: "Yes — every action in Acme is available over a typed API and an MCP server.",
  },
  {
    q: "Do you offer SSO?",
    a: "SSO and audit logging are included on the Business plan.",
  },
];
