import React from "react";
import { Link, Image } from "@pylonsync/react";

export default function ScalingPost() {
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
          Engineering · April 2026
        </span>
        <h1 className="display mt-3 text-4xl text-[var(--color-ink)] sm:text-5xl">
          Scaling Acme to 50,000 teams on a single Rust binary.
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
            <p className="text-sm font-medium text-[var(--color-ink)]">
              James Patel
            </p>
            <p className="text-xs text-[var(--color-stone)]">
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
        <p className="text-lg leading-relaxed text-[var(--color-ink-soft)]">
          When we tell people that Acme runs its entire production fleet on six
          machines, they assume we're being cute. We're not — and the reason
          is unromantic. We picked boring tools, kept the architecture small,
          and let cache-line discipline do the heavy lifting.
        </p>

        <h2 className="display mt-12 text-2xl text-[var(--color-ink)]">
          One binary, one port.
        </h2>
        <p className="mt-4 leading-relaxed text-[var(--color-stone)]">
          Every Acme deploy is a single Rust binary. It serves the SSR pages,
          the JSON API, the WebSocket fanout, the background workers, and the
          image optimization endpoint — all on the same port. There is no
          message broker. There is no separate API server. There is no Sidekiq
          equivalent.
        </p>
        <p className="mt-4 leading-relaxed text-[var(--color-stone)]">
          We get away with this because the per-request cost in Rust is so low
          that we don't need to amortize it across a dedicated service. The
          binary is small enough to fit in instruction cache. Hot paths stay
          hot.
        </p>

        <h2 className="display mt-12 text-2xl text-[var(--color-ink)]">
          Cache-line discipline.
        </h2>
        <p className="mt-4 leading-relaxed text-[var(--color-stone)]">
          The biggest win — bigger than the choice of language, bigger than the
          choice of database — was getting our hot data structures aligned so
          that an L2 miss became a notable event instead of a constant. Every
          time we drop a microsecond off the request path, the math gets better.
        </p>

        <h2 className="display mt-12 text-2xl text-[var(--color-ink)]">
          What we're not doing.
        </h2>
        <p className="mt-4 leading-relaxed text-[var(--color-stone)]">
          We're not microservicing. We're not Kafka-ing. We're not
          Kubernetes-ing. We deploy with <code>scp</code> and run with
          systemd. The boring choice keeps winning.
        </p>
      </article>

      <hr className="mt-16 border-[var(--color-line)]" />
      <div className="mt-12 flex items-center justify-between">
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
