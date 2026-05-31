import React from "react";
import { Link, Image } from "@pylonsync/react";

export default function AboutPage() {
  return (
    <main>
      <section className="mx-auto max-w-3xl px-6 pt-20 pb-16 md:pt-28">
        <span className="text-xs font-medium uppercase tracking-wider text-[var(--color-brand-deep)]">
          About
        </span>
        <h1 className="display mt-3 text-4xl text-[var(--color-ink)] sm:text-6xl">
          Software for the work you actually do.
        </h1>
        <p className="mt-8 max-w-2xl text-lg leading-relaxed text-[var(--color-stone)]">
          We started Acme in 2024 because we kept opening the same six tabs
          every morning and closing them every evening, and nothing in between
          felt like it was on our side. The category we wanted didn't exist, so
          we wrote it.
        </p>
        <p className="mt-6 max-w-2xl text-lg leading-relaxed text-[var(--color-stone)]">
          Acme is a team of twelve in Dallas, Austin, and the Pacific Northwest.
          We ship every Friday, dogfood everything, and pay the mortgage on
          quiet competence — no growth hacks, no demo-day theatrics, no fake
          urgency in the pricing page.
        </p>
      </section>

      <section className="mx-auto max-w-6xl px-6 py-24">
        <h2 className="display text-3xl text-[var(--color-ink)] sm:text-4xl">
          What we believe.
        </h2>
        <div className="mt-12 grid grid-cols-1 gap-12 md:grid-cols-3">
          {BELIEFS.map((b) => (
            <div key={b.title}>
              <span className="text-xs font-medium uppercase tracking-wider text-[var(--color-brand-deep)]">
                {b.eyebrow}
              </span>
              <h3 className="mt-3 text-lg font-semibold text-[var(--color-ink)]">
                {b.title}
              </h3>
              <p className="mt-3 text-sm leading-relaxed text-[var(--color-stone)]">
                {b.body}
              </p>
            </div>
          ))}
        </div>
      </section>

      <section className="bg-[var(--color-cream-deep)]/40">
        <div className="mx-auto max-w-6xl px-6 py-24">
          <h2 className="display text-3xl text-[var(--color-ink)] sm:text-4xl">
            The team.
          </h2>
          <p className="mt-4 max-w-xl text-[var(--color-stone)]">
            Twelve people, no managers of managers. Hire the best you can find,
            give them the room to ship.
          </p>
          <div className="mt-12 grid grid-cols-2 gap-8 sm:grid-cols-3 md:grid-cols-4">
            {TEAM.map((p) => (
              <div key={p.name}>
                <Image
                  src={p.src}
                  alt=""
                  width={400}
                  height={400}
                  sizes="(max-width: 768px) 50vw, 240px"
                  className="aspect-square w-full rounded-2xl object-cover"
                />
                <p className="mt-4 text-sm font-medium text-[var(--color-ink)]">
                  {p.name}
                </p>
                <p className="text-xs text-[var(--color-stone)]">{p.role}</p>
              </div>
            ))}
          </div>
        </div>
      </section>

      <section className="mx-auto max-w-3xl px-6 py-24 text-center">
        <h2 className="display text-3xl text-[var(--color-ink)] sm:text-4xl">
          We're hiring.
        </h2>
        <p className="mt-6 text-lg text-[var(--color-stone)]">
          Designers and senior engineers who'd rather ship five things and
          delete two than ship a hundred and own none of them.
        </p>
        <Link
          href="/careers"
          className="mt-10 inline-flex items-center gap-2 rounded-full bg-[var(--color-ink)] px-6 py-3 text-sm font-medium text-[var(--color-cream)] transition hover:bg-[var(--color-ink-soft)]"
        >
          Open roles →
        </Link>
      </section>
    </main>
  );
}

const BELIEFS = [
  {
    eyebrow: "01",
    title: "The fewer tabs, the better.",
    body: "Every new window your team opens is a small tax on their attention. We charge ourselves that tax aggressively. Acme replaces six tabs, not three.",
  },
  {
    eyebrow: "02",
    title: "Speed is a feature.",
    body: "If it takes more than 30ms, we treat it as a bug, not a tradeoff. We rewrote our backend in Rust because the alternative was watching loading spinners ship.",
  },
  {
    eyebrow: "03",
    title: "Pricing is a promise.",
    body: "You won't see a 'contact us' for anything under enterprise. You won't see a 'we ran out of funding' email next quarter. Honest prices, honest road map, honest year-end.",
  },
];

const TEAM = [
  { name: "Mara Chen", role: "Co-founder, CEO", src: "/avatar-1.jpg" },
  { name: "James Patel", role: "Co-founder, CTO", src: "/avatar-2.jpg" },
  { name: "Eira Nilsson", role: "Design", src: "/avatar-3.jpg" },
  { name: "Marcus Okafor", role: "Engineering", src: "/avatar-4.jpg" },
];
