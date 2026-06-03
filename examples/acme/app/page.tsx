import React from "react";
import { Link } from "@pylonsync/react";

// Acme — an original landing page rendered entirely with CSS (no product
// screenshots). It exists to show off Pylon's server-side rendering, file-
// based routes, and instant <Link> navigation, not to resemble any real
// product.
export default function HomePage() {
  return (
    <main className="mx-auto max-w-6xl px-6">
      <Hero />
      <FeatureTrio />
      <Metrics />
      <Workflow />
      <Testimonial />
      <Cta />
    </main>
  );
}

function Hero() {
  return (
    <section className="pt-24 pb-16 text-center md:pt-32">
      <span className="inline-flex items-center gap-2 rounded-full border border-[var(--color-line)] bg-white px-3 py-1 text-xs font-medium text-[var(--color-stone)]">
        <span className="h-1.5 w-1.5 rounded-full bg-[var(--color-accent)]" />
        New — automations are here
      </span>
      <h1 className="display mx-auto mt-7 max-w-4xl text-[52px] text-[var(--color-ink)] sm:text-[72px] md:text-[84px]">
        Your team's work,
        <br />
        finally in <span className="hl">one place</span>.
      </h1>
      <p className="mx-auto mt-8 max-w-xl text-lg text-[var(--color-muted)]">
        Projects, docs, and updates live together in Acme — so nothing slips
        between six different tabs, and everyone can see where things stand.
      </p>
      <div className="mt-10 flex items-center justify-center gap-3">
        <Link
          href="/sign-up"
          className="inline-flex items-center justify-center rounded-lg bg-[var(--color-ink)] px-5 py-2.5 text-sm font-medium text-white shadow-sm transition hover:bg-[var(--color-ink-soft)]"
        >
          Get started free
        </Link>
        <Link
          href="/pricing"
          className="inline-flex items-center justify-center rounded-lg border border-[var(--color-line)] bg-white px-5 py-2.5 text-sm font-medium text-[var(--color-ink)] transition hover:bg-[var(--color-cream-deep)]"
        >
          See pricing
        </Link>
      </div>

      {/* CSS-rendered product mockup — a fake app window, no screenshot. */}
      <div className="mt-16">
        <AppMock />
      </div>
    </section>
  );
}

/** A faux application window drawn entirely in CSS: title bar, a sidebar
 *  of "projects", and a small board of task cards. Original, self-
 *  contained, and never a real screenshot. */
function AppMock() {
  const projects = ["Website", "Mobile app", "Q3 launch", "Design system"];
  const columns = [
    { name: "Up next", cards: ["Audit onboarding", "Draft changelog"] },
    { name: "In progress", cards: ["Billing v2", "Search filters", "Dark mode"] },
    { name: "Done", cards: ["Import API"] },
  ];
  return (
    <div className="mock mx-auto max-w-4xl overflow-hidden text-left">
      <div className="flex items-center gap-2 border-b border-[var(--color-line)] px-4 py-3">
        <span className="h-3 w-3 rounded-full bg-[var(--color-line)]" />
        <span className="h-3 w-3 rounded-full bg-[var(--color-line)]" />
        <span className="h-3 w-3 rounded-full bg-[var(--color-line)]" />
        <span className="ml-3 rounded-md bg-[var(--color-cream-deep)] px-2 py-0.5 text-xs text-[var(--color-stone)]">
          acme.app / board
        </span>
      </div>
      <div className="grid grid-cols-[150px_1fr]">
        <aside className="hidden border-r border-[var(--color-line)] p-4 sm:block">
          <div className="mb-4 flex items-center gap-2 text-xs font-semibold text-[var(--color-ink)]">
            <span className="h-4 w-4 rounded bg-[var(--color-accent)]" />
            Acme
          </div>
          {projects.map((p, i) => (
            <div
              key={p}
              className={
                "mb-1 flex items-center gap-2 rounded-md px-2 py-1.5 text-xs " +
                (i === 1
                  ? "bg-[var(--color-accent-soft)] text-[var(--color-accent-deep)]"
                  : "text-[var(--color-stone)]")
              }
            >
              <span className="h-1.5 w-1.5 rounded-full bg-current opacity-60" />
              {p}
            </div>
          ))}
        </aside>
        <div className="grid grid-cols-3 gap-3 bg-[var(--color-cream)] p-4">
          {columns.map((col) => (
            <div key={col.name}>
              <div className="mb-2 flex items-center justify-between text-[10px] font-semibold uppercase tracking-wider text-[var(--color-stone)]">
                <span>{col.name}</span>
                <span>{col.cards.length}</span>
              </div>
              <div className="space-y-2">
                {col.cards.map((c) => (
                  <div
                    key={c}
                    className="rounded-lg border border-[var(--color-line)] bg-white p-2.5 shadow-sm"
                  >
                    <div className="text-xs font-medium text-[var(--color-ink)]">
                      {c}
                    </div>
                    <div className="mt-2 flex items-center gap-1.5">
                      <span className="h-2 w-8 rounded-full bg-[var(--color-cream-deep)]" />
                      <span className="ml-auto h-4 w-4 rounded-full bg-[var(--color-accent-soft)]" />
                    </div>
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

function FeatureTrio() {
  const items = [
    {
      title: "Projects",
      body: "Boards, lists, and timelines that stay in sync for everyone, the moment anything changes.",
    },
    {
      title: "Docs",
      body: "Write specs and notes right next to the work they describe — linked, searchable, never lost.",
    },
    {
      title: "Automations",
      body: "Move a card, notify a channel, open a follow-up. Wire up the routine so the team doesn't have to.",
    },
  ];
  return (
    <section className="grid grid-cols-1 gap-10 py-24 md:grid-cols-3">
      {items.map((it) => (
        <div key={it.title}>
          <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-[var(--color-accent-soft)]">
            <span className="h-3.5 w-3.5 rounded-md bg-[var(--color-accent)]" />
          </div>
          <h3 className="mt-5 text-lg font-semibold text-[var(--color-ink)]">
            {it.title}
          </h3>
          <p className="mt-2 text-sm leading-relaxed text-[var(--color-muted)]">
            {it.body}
          </p>
        </div>
      ))}
    </section>
  );
}

function Metrics() {
  const stats = [
    { value: "4,000+", label: "teams run on Acme" },
    { value: "1.2M", label: "tasks shipped weekly" },
    { value: "99.99%", label: "uptime, last 12 months" },
    { value: "12 min", label: "median setup time" },
  ];
  return (
    <section className="rounded-3xl bg-[var(--color-card)] px-6 py-14 sm:px-12">
      <div className="grid grid-cols-2 gap-8 md:grid-cols-4">
        {stats.map((s) => (
          <div key={s.label}>
            <div className="display text-4xl text-[var(--color-ink)] sm:text-5xl">
              {s.value}
            </div>
            <div className="mt-2 text-sm text-[var(--color-muted)]">
              {s.label}
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}

function Workflow() {
  const steps = [
    { n: "01", title: "Bring it together", body: "Import from wherever your work lives today. Acme keeps the structure you already have." },
    { n: "02", title: "Make it move", body: "Add a couple of automations and the busy parts run themselves — assignments, reminders, handoffs." },
    { n: "03", title: "See the whole picture", body: "One view across every project, so status meetings turn into a glance." },
  ];
  return (
    <section className="py-24">
      <h2 className="display max-w-2xl text-[40px] text-[var(--color-ink)] sm:text-[52px]">
        Set it up once.
        <br />
        Then get out of the way.
      </h2>
      <div className="mt-14 grid grid-cols-1 gap-8 md:grid-cols-3">
        {steps.map((s) => (
          <div key={s.n} className="border-t border-[var(--color-line)] pt-6">
            <span className="text-sm font-semibold text-[var(--color-accent-deep)]">
              {s.n}
            </span>
            <h3 className="mt-3 text-lg font-semibold text-[var(--color-ink)]">
              {s.title}
            </h3>
            <p className="mt-2 text-sm leading-relaxed text-[var(--color-muted)]">
              {s.body}
            </p>
          </div>
        ))}
      </div>
    </section>
  );
}

function Testimonial() {
  return (
    <section className="pb-24">
      <div className="rounded-3xl bg-[var(--color-card)] p-8 sm:p-12">
        <div className="flex items-center gap-4">
          <span className="flex h-12 w-12 items-center justify-center rounded-full bg-[var(--color-accent)] text-sm font-semibold text-white">
            RM
          </span>
          <div>
            <p className="text-sm font-medium text-[var(--color-ink)]">
              Riya Mehta
            </p>
            <p className="text-xs text-[var(--color-muted)]">
              Head of Operations, Northwind Studio
            </p>
          </div>
        </div>
        <blockquote className="mt-8 text-2xl leading-snug text-[var(--color-ink)] sm:text-3xl">
          "We replaced four tools with Acme and stopped losing track of work
          between them. The whole team can finally see the same thing."
        </blockquote>
      </div>
    </section>
  );
}

function Cta() {
  return (
    <section className="pb-32 text-center">
      <h2 className="display mx-auto max-w-3xl text-[48px] text-[var(--color-ink)] sm:text-[72px]">
        Get your team
        <br />
        on the <span className="hl">same page</span>.
      </h2>
      <p className="mx-auto mt-8 max-w-lg text-lg text-[var(--color-muted)]">
        Acme sets up in about twelve minutes. Free for your first three
        projects — no credit card.
      </p>
      <div className="mt-10">
        <Link
          href="/sign-up"
          className="inline-flex items-center justify-center rounded-lg bg-[var(--color-ink)] px-5 py-2.5 text-sm font-medium text-white shadow-sm transition hover:bg-[var(--color-ink-soft)]"
        >
          Get started free
        </Link>
      </div>
    </section>
  );
}
