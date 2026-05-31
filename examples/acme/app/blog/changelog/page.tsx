import React from "react";
import { Link, Image } from "@pylonsync/react";

interface PageProps {
  url: string;
}

const ITEMS = [
  {
    badge: "New",
    title: "Command palette, everywhere.",
    body: "Cmd-K anywhere in Acme to jump, search, run, or compose. We rewrote the whole indexing layer from scratch in the process.",
  },
  {
    badge: "Faster",
    title: "Search is 4x quicker.",
    body: "Median search time is now 28ms, p99 is 80ms. We re-tuned the BM25 weights based on what queries actually look like in production.",
  },
  {
    badge: "New",
    title: "Recurring tasks, finally.",
    body: "Daily, weekly, monthly, custom-cron. Tied into the same timer system that runs scheduled actions.",
  },
  {
    badge: "Better",
    title: "Slack integration v2.",
    body: "Bidirectional reactions. Threaded replies stay threaded. The 'someone unfurled your link in a different channel' problem is solved.",
  },
  {
    badge: "Removed",
    title: "The duplicate-tab problem on macOS.",
    body: "If you open Acme in two tabs, you'll get a 'switch to the existing one' prompt instead of a confused second session.",
  },
  {
    badge: "Better",
    title: "Audit log filtering.",
    body: "Filter by actor, action, target, IP. Export to CSV from inside the UI — no API trip required.",
  },
];

export default function ChangelogPost({ url }: PageProps) {
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
          Changelog · April 2026
        </span>
        <h1 className="display mt-3 text-4xl text-[var(--color-ink)] sm:text-5xl">
          April changelog: 23 things we shipped while you were busy.
        </h1>
        <div className="mt-8 flex items-center gap-3">
          <Image
            src="/avatar-3.jpg"
            alt=""
            width={64}
            height={64}
            className="h-10 w-10 rounded-full object-cover"
          />
          <div>
            <p className="text-sm font-medium text-[var(--color-ink)]">
              Eira Nilsson
            </p>
            <p className="text-xs text-[var(--color-stone)]">
              Design · 3 min read
            </p>
          </div>
        </div>
      </div>

      <Image
        src="/blog-3.jpg"
        alt=""
        width={1600}
        height={900}
        sizes="(max-width: 768px) 100vw, 720px"
        className="mt-12 aspect-[16/9] w-full rounded-2xl object-cover"
        priority
      />

      <ul className="mt-12 divide-y divide-[var(--color-line)] border-y border-[var(--color-line)]">
        {ITEMS.map((item, i) => (
          <li key={i} className="py-6">
            <div className="flex items-start gap-4">
              <span
                className={
                  "mt-0.5 inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-medium " +
                  (item.badge === "New"
                    ? "bg-[var(--color-brand)] text-[var(--color-cream)]"
                    : item.badge === "Removed"
                      ? "bg-stone-200 text-[var(--color-stone)]"
                      : "bg-[var(--color-cream-deep)] text-[var(--color-ink-soft)]")
                }
              >
                {item.badge}
              </span>
              <div className="flex-1">
                <h2 className="text-lg font-semibold tracking-tight text-[var(--color-ink)]">
                  {item.title}
                </h2>
                <p className="mt-2 text-sm leading-relaxed text-[var(--color-stone)]">
                  {item.body}
                </p>
              </div>
            </div>
          </li>
        ))}
      </ul>

      <p className="mt-12 text-sm text-[var(--color-stone)]">
        You're on <code className="text-[var(--color-ink)]">{url}</code>. Every
        change above ships to your account today — we don't gate features
        behind release schedules.
      </p>

      <div className="mt-16 flex items-center justify-between">
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
