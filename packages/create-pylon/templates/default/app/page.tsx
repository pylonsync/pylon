import React from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";

// SEO metadata. Exported `metadata` is rendered into <head> on the server, so
// this marketing page is fully indexable — view source and the copy is in the
// HTML. Swap "Acme" for your product throughout.
export const metadata: Metadata = {
  title: "Acme — the workspace where work gets done",
  description:
    "Acme brings your projects, your people, and your updates into one fast, real-time workspace. Plan together, ship together, keep everyone in the loop.",
};

// Shared container width. The whole page is a contained, left-aligned column.
const WRAP = "mx-auto w-full max-w-5xl px-6";

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
        <Badge>New · Acme for teams is here</Badge>
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
                title="Logo placeholder"
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
          <Portrait />
          <figure>
            <div className="text-4xl font-serif leading-none text-brand">
              &ldquo;
            </div>
            <blockquote className="mt-4 text-balance text-2xl font-medium leading-[1.35] tracking-[-0.01em] sm:text-[1.75rem]">
              Our whole team finally works in one place. Acme made it easy to see
              what is happening, decide what is next, and keep everyone moving in
              the same direction.
            </blockquote>
            <figcaption className="mt-8 border-t border-zinc-200/70 pt-6">
              <div className="text-sm font-semibold">Placeholder Name</div>
              <div className="text-sm text-zinc-500">Head of Product, Northwind</div>
            </figcaption>
          </figure>
        </div>
      </section>

      {/* =================== FEATURE: PROJECTS ======================= */}
      <Divider />
      <section id="product" className={`${WRAP} py-20`}>
        <SectionHead
          eyebrow="Projects"
          arrow
          title="A project board your team actually uses."
          body="Give everyone one simple place to plan and track the work, and a board that stays organized on its own as things move."
        />
        <FeatureGrid className="mt-14" items={PROJECT_FEATURES} />
        <div className="mt-16">
          <Shot url="acme.app/projects" label="Projects board" />
        </div>
      </section>

      {/* ===================== ENTRY POINTS ========================= */}
      <Divider />
      <section className={`${WRAP} py-20`}>
        <SectionHead
          eyebrow="Anywhere"
          title="Meet your team where they already are."
          body="Both entry points share the same projects, roadmap, and updates underneath, so nothing lives in two places."
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

      {/* ===================== FEATURE: ROADMAP ===================== */}
      <Divider />
      <section className={`${WRAP} py-20`}>
        <SectionHead
          eyebrow="Roadmap"
          arrow
          title="A roadmap that updates itself as you work."
          body="The public view stays in sync with the work your team is actually doing. Move a card and the roadmap reflects it instantly."
        />
        <FeatureGrid className="mt-14" items={ROADMAP_FEATURES} />
        <div className="mt-16">
          <Shot url="acme.app/roadmap" label="Roadmap view" />
        </div>
      </section>

      {/* ===================== FEATURE: UPDATES ===================== */}
      <Divider />
      <section className={`${WRAP} py-20`}>
        <SectionHead
          eyebrow="Updates"
          arrow
          title="Updates your team will not miss."
          body="Turn every shipped change into an update that reaches the people who asked for it, without extra work from your team."
        />
        <FeatureGrid className="mt-14" items={UPDATE_FEATURES} />
        <div className="mt-16">
          <Shot url="acme.app/updates" label="Changelog composer" />
        </div>
      </section>

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

      {/* ======================= AI AGENTS ========================= */}
      <Divider />
      <section className={`${WRAP} py-20`}>
        <SectionHead
          eyebrow="AI agents"
          arrow
          title="Run it through the agent you already use."
          body="Acme ships with an MCP server, so Claude, Cursor, or any agent you work with can triage work, update the roadmap, and draft updates from wherever you already are."
        />
        <div className="mt-6">
          <GhostLink href="/#product">Learn more →</GhostLink>
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
              key={q.name}
              className="flex flex-col rounded-2xl border border-zinc-200 bg-paper p-6"
            >
              <blockquote className="text-[14px] leading-relaxed text-zinc-600">
                &ldquo;{q.quote}&rdquo;
              </blockquote>
              <figcaption className="mt-6 flex items-center gap-3">
                <span className="size-8 rounded-full bg-zinc-200" />
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
                  <PrimaryButton href={primaryHref} className="w-full justify-center">
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
          <PrimaryButton href={primaryHref}>Start building with Acme</PrimaryButton>
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

/* ---------------------------------------------------------------- */
/* Reusable bits                                                     */
/* ---------------------------------------------------------------- */

function Divider() {
  return (
    <div className={WRAP}>
      <div className="border-t border-zinc-200/70" />
    </div>
  );
}

function Eyebrow({ children }: { children: React.ReactNode }) {
  return (
    <p className="font-mono text-[11px] font-semibold uppercase tracking-[0.14em] text-brand">
      {children}
    </p>
  );
}

function Badge({ children }: { children: React.ReactNode }) {
  return (
    <span className="inline-flex items-center gap-2 rounded-full border border-zinc-200 bg-white py-1 pl-1 pr-3 text-[13px] text-zinc-600">
      <span className="rounded-full bg-brand-soft px-2 py-0.5 text-[11px] font-semibold uppercase tracking-wide text-brand">
        New
      </span>
      Acme for teams is here →
    </span>
  );
}

function SectionHead({
  eyebrow,
  title,
  body,
  arrow,
}: {
  eyebrow: string;
  title: string;
  body: string;
  arrow?: boolean;
}) {
  return (
    <div>
      <Eyebrow>
        {eyebrow}
        {arrow ? " →" : ""}
      </Eyebrow>
      <h2 className="mt-4 max-w-2xl text-balance text-3xl font-semibold leading-[1.1] tracking-[-0.02em] sm:text-[2.5rem]">
        {title}
      </h2>
      <p className="mt-5 max-w-xl text-[15px] leading-relaxed text-zinc-500">
        {body}
      </p>
    </div>
  );
}

function FeatureGrid({
  items,
  columns = 3,
  className = "",
}: {
  items: { title: string; body: string }[];
  columns?: 2 | 3;
  className?: string;
}) {
  const cols = columns === 2 ? "sm:grid-cols-2" : "sm:grid-cols-2 lg:grid-cols-3";
  return (
    <div className={`grid gap-x-8 gap-y-10 ${cols} ${className}`}>
      {items.map((f) => (
        <div key={f.title}>
          <h3 className="text-[15px] font-medium text-brand">{f.title}</h3>
          <p className="mt-2 text-[14px] leading-relaxed text-zinc-500">
            {f.body}
          </p>
        </div>
      ))}
    </div>
  );
}

function PrimaryButton({
  href,
  children,
  className = "",
}: {
  href: string;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <Link
      href={href}
      className={`inline-flex items-center rounded-full bg-zinc-900 px-5 py-2.5 text-sm font-medium text-white transition-colors hover:bg-zinc-700 ${className}`}
    >
      {children}
    </Link>
  );
}

function GhostLink({
  href,
  children,
}: {
  href: string;
  children: React.ReactNode;
}) {
  return (
    <Link
      href={href}
      className="text-sm font-medium text-zinc-700 transition-colors hover:text-zinc-900"
    >
      {children}
    </Link>
  );
}

// Browser-chrome frame around an image placeholder. Drop a real screenshot in
// place of the dashed box.
function Shot({ url, label }: { url: string; label: string }) {
  return (
    <div className="overflow-hidden rounded-2xl border border-zinc-200 bg-white shadow-[0_30px_70px_-35px_rgba(0,0,0,0.3)]">
      <div className="flex items-center gap-1.5 border-b border-zinc-100 px-4 py-3">
        <span className="size-2.5 rounded-full bg-zinc-200" />
        <span className="size-2.5 rounded-full bg-zinc-200" />
        <span className="size-2.5 rounded-full bg-zinc-200" />
        <span className="mx-auto rounded-md bg-zinc-100 px-10 py-1 text-[11px] text-zinc-400">
          {url}
        </span>
      </div>
      <div className="grid aspect-[16/9] place-items-center bg-zinc-50">
        <div className="flex flex-col items-center gap-2.5 text-zinc-400">
          <span className="flex size-11 items-center justify-center rounded-xl border-2 border-dashed border-zinc-300 text-lg">
            ▦
          </span>
          <p className="text-sm font-medium text-zinc-500">{label}</p>
          <p className="text-xs text-zinc-400">Replace with a screenshot</p>
        </div>
      </div>
    </div>
  );
}

function Portrait() {
  return (
    <div className="grid aspect-square w-full place-items-center overflow-hidden rounded-2xl border border-zinc-200 bg-zinc-100">
      <div className="flex flex-col items-center gap-2 text-zinc-400">
        <span className="flex size-12 items-center justify-center rounded-full border-2 border-dashed border-zinc-300 text-xl">
          ◐
        </span>
        <p className="text-xs">Portrait placeholder</p>
      </div>
    </div>
  );
}

function Terminal() {
  return (
    <div className="overflow-hidden rounded-2xl border border-zinc-800 bg-zinc-950 shadow-[0_30px_70px_-35px_rgba(0,0,0,0.6)]">
      <div className="flex items-center gap-1.5 border-b border-white/10 px-4 py-3">
        <span className="size-2.5 rounded-full bg-white/20" />
        <span className="size-2.5 rounded-full bg-white/20" />
        <span className="size-2.5 rounded-full bg-white/20" />
        <span className="ml-3 font-mono text-[11px] text-white/40">
          agent — acme-mcp
        </span>
      </div>
      <div className="space-y-3 p-6 font-mono text-[12.5px] leading-relaxed text-zinc-300">
        <p className="text-zinc-500">› Draft an update for what shipped this week.</p>
        <p className="text-brand">
          acme-mcp::listShipped <span className="text-zinc-500">→ 6 items across 2 projects</span>
        </p>
        <p className="text-brand">
          acme-mcp::draftUpdate <span className="text-zinc-500">→ draft ready</span>
        </p>
        <div className="rounded-lg border-l-2 border-brand/60 bg-white/5 px-4 py-3 text-zinc-200">
          <p className="font-semibold">This week in Acme</p>
          <p className="mt-1 text-zinc-400">
            Faster search, bulk actions on the board, and a fix for threaded
            comments. [placeholder copy]
          </p>
        </div>
        <p className="text-zinc-500">› Schedule it for 9am tomorrow.</p>
        <p className="text-brand">
          acme-mcp::scheduleUpdate{" "}
          <span className="text-zinc-500">→ queued, notifies your team</span>
        </p>
      </div>
    </div>
  );
}

/* ---------------------------------------------------------------- */
/* Content (all placeholder — swap for your own)                    */
/* ---------------------------------------------------------------- */

const OUTCOMES = [
  {
    title: "Work in one place",
    body: "Plans, tasks, and updates live together, so nothing gets lost between tools.",
  },
  {
    title: "Priorities in the open",
    body: "Everyone can see what is planned, in progress, and shipped — and why.",
  },
  {
    title: "Updates that land",
    body: "Turn every release into a note that reaches the people who care.",
  },
  {
    title: "A team that stays in sync",
    body: "Real-time by default, so the board is the same on every screen.",
  },
];

const PROJECT_FEATURES = [
  {
    title: "Public or private boards",
    body: "Make a board visible to everyone, or keep it between your team.",
  },
  {
    title: "Comments and reactions",
    body: "Discuss the work where it lives, with the context attached.",
  },
  {
    title: "Smart organization",
    body: "New items are sorted and tagged on the way in, so the board stays clean.",
  },
  {
    title: "Guest contributions",
    body: "Let people add ideas without an account, while your team stays in control.",
  },
  {
    title: "Automatic tagging",
    body: "Every item is categorized so you can filter and search without setup.",
  },
  {
    title: "Embeddable widget",
    body: "Drop a single snippet into your product and the board opens right there.",
  },
];

const ENTRY_POINTS = [
  {
    icon: "▢",
    title: "Standalone workspace",
    body: "A branded page where your whole team browses, votes, and adds work. No code required, just a link.",
  },
  {
    icon: "◫",
    title: "In-app widget",
    body: "One snippet puts projects, roadmap, and updates inside your product, so active users can weigh in without leaving.",
  },
];

const ROADMAP_FEATURES = [
  {
    title: "Always in sync",
    body: "Move a card and the public roadmap updates immediately. No duplicate work.",
  },
  {
    title: "Votes on every item",
    body: "Each item shows how many people want it, so demand sits next to direction.",
  },
  {
    title: "Linked to the work",
    body: "Every item links back to the idea it came from, giving people credit for the ask.",
  },
  {
    title: "A clear lifecycle",
    body: "Planned, in progress, done. A simple path every item follows.",
  },
  {
    title: "Built-in guardrails",
    body: "Gentle warnings when too much piles up keep the roadmap focused and honest.",
  },
  {
    title: "Stale item detection",
    body: "Items untouched for too long get flagged, so you commit or close them cleanly.",
  },
];

const UPDATE_FEATURES = [
  {
    title: "Drafts, written for you",
    body: "Closed work becomes a ready-to-edit update, so you never start from a blank page.",
  },
  {
    title: "Reaches the right people",
    body: "Every update reaches the people who asked for it, so they hear first.",
  },
  {
    title: "In-app indicator",
    body: "Active users see a fresh-update badge the moment you publish, no email required.",
  },
  {
    title: "Linked to requests",
    body: "Each update references the work that sparked it, so anyone can trace it back.",
  },
  {
    title: "Scheduled publishing",
    body: "Queue updates in advance so announcements go live exactly when you want.",
  },
  {
    title: "Smart prompts",
    body: "Reminders nudge your team to publish when there is enough shipped work worth sharing.",
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
    name: "Placeholder Name",
    role: "Founder, Globex",
  },
  {
    quote:
      "Acme noticeably improved how we plan. Instead of piecing together five tools, we have one hub for everything.",
    name: "Placeholder Name",
    role: "Founder, OpenLane",
  },
  {
    quote:
      "Ever since we added Acme, people actually feel heard. It has helped us build a loyal community of users.",
    name: "Placeholder Name",
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
    a: "Acme works for any team that plans and ships work together — product, design, engineering, or ops. [Placeholder answer.]",
  },
  {
    q: "What about migrating from another tool?",
    a: "Import your existing projects and pick up where you left off. [Placeholder answer.]",
  },
  {
    q: "Is there an API or a way to script this?",
    a: "Yes — every action in Acme is available over a typed API and an MCP server. [Placeholder answer.]",
  },
  {
    q: "Do you offer SSO?",
    a: "SSO and audit logging are included on the Business plan. [Placeholder answer.]",
  },
];
