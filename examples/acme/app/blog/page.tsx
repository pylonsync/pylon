import React from "react";
import { Link } from "@pylonsync/react";

const POSTS = [
  {
    slug: "our-series-a",
    title: "Acme raises $24M to keep teams' work in one place.",
    excerpt:
      "We've raised a Series A from Index, with participation from our existing investors. Here's exactly what we're going to do with it.",
    grad: "from-[var(--color-accent-soft)] to-[var(--color-cream-deep)]",
    author: "Mara Chen",
    initials: "MC",
    date: "May 2026",
    tag: "Company",
  },
  {
    slug: "scaling-fast",
    title: "What we learned scaling Acme from 100 to 50,000 teams.",
    excerpt:
      "We refused to hire ahead of revenue. We deleted features. We deprecated integrations our biggest customers didn't use. Here's what changed, what didn't, and the calls we'd defend.",
    grad: "from-[#dfe9ff] to-[var(--color-cream-deep)]",
    author: "James Patel",
    initials: "JP",
    date: "April 2026",
    tag: "Engineering",
  },
  {
    slug: "changelog",
    title: "April changelog: 23 things we shipped while you were busy.",
    excerpt:
      "New: a command palette across every surface. Faster: search is 4× quicker. Removed: the duplicate-tab problem on macOS.",
    grad: "from-[#ffe7d6] to-[var(--color-cream-deep)]",
    author: "Eira Nilsson",
    initials: "EN",
    date: "April 2026",
    tag: "Changelog",
  },
];

export default function BlogPage() {
  const [feature, ...rest] = POSTS;
  return (
    <main className="mx-auto max-w-6xl px-6 pt-20 pb-24 md:pt-28">
      <span className="text-xs font-medium uppercase tracking-wider text-[var(--color-brand-deep)]">
        Blog
      </span>
      <h1 className="display mt-3 text-4xl text-[var(--color-ink)] sm:text-6xl">
        Writing, shipping, and the occasional tangent.
      </h1>

      <Link
        href={`/blog/${feature.slug}`}
        className="group mt-16 block overflow-hidden rounded-3xl border border-[var(--color-line)] bg-white transition hover:-translate-y-0.5 hover:shadow-md md:grid md:grid-cols-2"
      >
        <div
          className={
            "aspect-[16/9] w-full bg-gradient-to-br md:aspect-auto md:h-full " +
            feature.grad
          }
        />
        <div className="flex flex-col justify-between p-8 md:p-10">
          <div>
            <span className="text-xs font-medium uppercase tracking-wider text-[var(--color-brand-deep)]">
              {feature.tag} · {feature.date}
            </span>
            <h2 className="mt-4 text-3xl font-semibold tracking-tight text-[var(--color-ink)] md:text-4xl">
              {feature.title}
            </h2>
            <p className="mt-4 text-base leading-relaxed text-[var(--color-stone)]">
              {feature.excerpt}
            </p>
          </div>
          <p className="mt-8 inline-flex items-center gap-2 text-sm font-medium text-[var(--color-ink)]">
            By {feature.author}
            <span aria-hidden className="transition group-hover:translate-x-0.5">
              →
            </span>
          </p>
        </div>
      </Link>

      <div className="mt-12 grid grid-cols-1 gap-8 md:grid-cols-2">
        {rest.map((p) => (
          <Link
            key={p.slug}
            href={`/blog/${p.slug}`}
            className="group block overflow-hidden rounded-2xl border border-[var(--color-line)] bg-white transition hover:-translate-y-0.5 hover:shadow-md"
          >
            <div className={"aspect-[16/9] w-full bg-gradient-to-br " + p.grad} />
            <div className="p-6">
              <span className="text-xs font-medium uppercase tracking-wider text-[var(--color-brand-deep)]">
                {p.tag} · {p.date}
              </span>
              <h3 className="mt-3 text-xl font-semibold tracking-tight text-[var(--color-ink)]">
                {p.title}
              </h3>
              <p className="mt-3 text-sm leading-relaxed text-[var(--color-stone)]">
                {p.excerpt}
              </p>
              <p className="mt-6 text-xs text-[var(--color-stone)]">
                By {p.author}
              </p>
            </div>
          </Link>
        ))}
      </div>
    </main>
  );
}
