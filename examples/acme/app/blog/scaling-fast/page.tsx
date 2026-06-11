import React from "react";
import { Link, Image } from "@pylonsync/react";
import type { Metadata } from "@pylonsync/react";

export const metadata: Metadata = {
  title: "Scaling from 100 to 50k teams — Acme Blog",
  description:
    "What we learned scaling Acme from 100 to 50,000 teams: refusing to hire ahead of revenue, deleting features, and betting on boring.",
};

export default function ScalingPost() {
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
          Engineering · April 2026
        </span>
        <h1 className="display mt-3 text-4xl text-foreground sm:text-5xl">
          What we learned scaling Acme from 100 to 50,000 teams.
        </h1>
        <div className="mt-8 flex items-center gap-3">
          <Image
            src="/avatar-2.jpg"
            alt=""
            width={64}
            height={64}
            className="h-10 w-10 rounded-full object-cover"
          />
          <div>
            <p className="text-sm font-medium text-foreground">James Patel</p>
            <p className="text-xs text-muted-foreground">
              Co-founder, CTO · 8 min read
            </p>
          </div>
        </div>
      </div>

      <Image
        src="/blog-2.jpg"
        alt=""
        width={1600}
        height={900}
        sizes="(max-width: 768px) 100vw, 720px"
        className="mt-12 aspect-[16/9] w-full rounded-2xl object-cover"
        priority
      />

      <article className="prose mt-12 max-w-none">
        <p className="text-lg leading-relaxed text-foreground">
          Two years ago we had a hundred teams using Acme and a single shared
          Slack channel where I personally answered every question. Last week we
          crossed fifty thousand. Here's what changed, what didn't, and the
          decisions we'd defend at gunpoint.
        </p>

        <h2 className="display mt-12 text-2xl text-foreground">
          We refused to grow the headcount before the product was good.
        </h2>
        <p className="mt-4 leading-relaxed text-muted-foreground">
          The default startup advice — hire ahead of revenue — was wrong for
          us. We stayed at eight people for the first eighteen months because
          we hadn't earned the right to add the ninth. When the product was
          finally good enough that users were doing our marketing for us, the
          hiring decisions got easier and the people we hired stayed.
        </p>

        <h2 className="display mt-12 text-2xl text-foreground">
          We deleted features. A lot of them.
        </h2>
        <p className="mt-4 leading-relaxed text-muted-foreground">
          The version of Acme our first hundred teams used had 41 distinct
          surfaces in the navigation. Today's version has nine. The bigger we
          got, the harder we leaned into "no" — and the funny thing about saying
          no is that the things that survive get used a hundred times more.
        </p>

        <h2 className="display mt-12 text-2xl text-foreground">
          We stopped trying to be everywhere.
        </h2>
        <p className="mt-4 leading-relaxed text-muted-foreground">
          For our first two years we had a Slack integration, a Microsoft Teams
          integration, a Discord integration, a Webex integration, and four
          half-finished CRM connectors. We deprecated all but the two our
          biggest customers actually used. Quarterly support volume dropped 18%
          the month we shipped the deprecations.
        </p>

        <h2 className="display mt-12 text-2xl text-foreground">
          We didn't move to a "scale" pricing model.
        </h2>
        <p className="mt-4 leading-relaxed text-muted-foreground">
          Every funded SaaS company eventually invents a usage-based add-on
          tier with vague-sounding "API calls" pricing. We refused to. The
          revenue would have been real; the customer trust we'd have spent to
          get it would have been more real.
        </p>

        <h2 className="display mt-12 text-2xl text-foreground">
          We bet hard on the boring choice.
        </h2>
        <p className="mt-4 leading-relaxed text-muted-foreground">
          Most of the durable wins we got at scale weren't novel architectures.
          They were the result of someone on our team taking a single boring
          thing — the way a particular query was indexed, the way one screen
          handled a missing field, the way a help article was titled — and
          making it 40% better. Compounded across a year, "40% better at the
          boring thing" beats every flashy decision we considered.
        </p>

        <h2 className="display mt-12 text-2xl text-foreground">
          What we'd do differently.
        </h2>
        <p className="mt-4 leading-relaxed text-muted-foreground">
          We'd hire an editor a year earlier. We'd kill our first attempt at a
          mobile app instead of carrying it for six quarters. And we'd stop
          pretending pricing experiments could be A/B tested — pricing is a
          relationship, not a knob.
        </p>
      </article>

      <hr className="mt-16 border-border" />
      <div className="mt-12 flex items-center justify-between">
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
