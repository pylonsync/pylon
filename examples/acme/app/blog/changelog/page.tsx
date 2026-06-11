import React from "react";
import { Link, Image } from "@pylonsync/react";
import type { Metadata } from "@pylonsync/react";

export const metadata: Metadata = {
  title: "April changelog — Acme",
  description: "23 things we shipped while you were busy.",
};

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

function badgeClasses(badge: string): string {
  if (badge === "New")
    return "bg-[var(--color-brand)] text-background";
  if (badge === "Removed")
    return "bg-secondary text-muted-foreground";
  return "bg-muted text-foreground";
}

export default function ChangelogPost({ url }: PageProps) {
  return (
    <main className="mx-auto max-w-3xl px-6 pt-16 pb-24 md:pt-24">
      <Link
        href="/blog"
        className="inline-flex items-center gap-1 text-sm text-muted-foreground transition hover:text-foreground"
      >
        ← Back to blog
      </Link>
      <div className="mt-8">
        <span className="text-xs font-medium uppercase tracking-wider text-[var(--color-brand-deep)]">
          Changelog · April 2026
        </span>
        <h1 className="display mt-3 text-4xl text-foreground sm:text-5xl">
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
            <p className="text-sm font-medium text-foreground">Eira Nilsson</p>
            <p className="text-xs text-muted-foreground">Design · 3 min read</p>
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

      <ul className="mt-12 divide-y divide-border border-y border-border">
        {ITEMS.map((item, i) => (
          <li key={i} className="py-6">
            <div className="flex items-start gap-4">
              <span
                className={
                  "mt-0.5 inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-medium " +
                  badgeClasses(item.badge)
                }
              >
                {item.badge}
              </span>
              <div className="flex-1">
                <h2 className="text-lg font-semibold tracking-tight text-foreground">
                  {item.title}
                </h2>
                <p className="mt-2 text-sm leading-relaxed text-muted-foreground">
                  {item.body}
                </p>
              </div>
            </div>
          </li>
        ))}
      </ul>

      <p className="mt-12 text-sm text-muted-foreground">
        You're on <code className="text-foreground">{url}</code>. Every change
        above ships to your account today — we don't gate features behind
        release schedules.
      </p>

      <div className="mt-16 flex items-center justify-between">
        <Link
          href="/blog"
          className="text-sm text-muted-foreground transition hover:text-foreground"
        >
          ← All posts
        </Link>
        <Link
          href="/contact"
          className="rounded-full bg-primary px-4 py-2 text-sm font-medium text-primary-foreground transition hover:opacity-90"
        >
          Try Acme →
        </Link>
      </div>
    </main>
  );
}
