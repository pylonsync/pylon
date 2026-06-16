"use client";

import React, { useEffect, useRef, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { EnsureGuest } from "@pylonsync/client";
import type { WaitlistConfig } from "@/lib/site.config";

// The interactive top of the landing page: the email-capture form and the LIVE
// signup counter. This is the realtime proof — the counter is a live
// `db.useQuery("WaitlistStat")` over the public, PII-free aggregate row, so the
// moment anyone (this tab or another) submits an email, joinWaitlist updates
// that row and the new count syncs to every open tab through the replica. No
// refresh, no polling.
//
// The signup form (joinWaitlist) is a public mutation, so it works for any
// anonymous visitor. The counter needs a live sync connection, so it's wrapped
// in <EnsureGuest>, which mints an anonymous guest session on first load — that
// session is what the sync engine connects with. It holds no PII, and the
// Signup table stays unreadable to it; the page only ever reads the aggregate
// count, never an email.

type HeroProps = {
  hero: WaitlistConfig["hero"];
  counter: WaitlistConfig["counter"];
};

export function WaitlistHero({ hero, counter }: HeroProps) {
  return (
    <div className="flex flex-col items-center text-center">
      <SignupForm hero={hero} />
      {counter.enabled ? (
        <div className="mt-8">
          <LiveCounter seed={counter.seedCount ?? 0} label={counter.label} />
        </div>
      ) : null}
    </div>
  );
}

/* ----------------------------- signup form ---------------------------- */

function SignupForm({ hero }: { hero: WaitlistConfig["hero"] }) {
  const [email, setEmail] = useState("");
  const [status, setStatus] = useState<"idle" | "sending" | "done">("idle");
  const [alreadyJoined, setAlreadyJoined] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    const value = email.trim();
    if (!value || status === "sending") return;
    setStatus("sending");
    setError(null);
    try {
      const res = await callFn<{ ok: boolean; alreadyJoined: boolean }>(
        "joinWaitlist",
        { email: value },
      );
      setAlreadyJoined(res.alreadyJoined);
      setStatus("done");
      // The live counter updates itself via the reactive push — no manual
      // increment needed here.
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(
        /valid email|INVALID_ARGS/i.test(msg)
          ? "Enter a valid email address."
          : "Something went wrong — try again in a moment.",
      );
      setStatus("idle");
    }
  }

  if (status === "done") {
    return (
      <div className="w-full max-w-md rounded-2xl border border-brand/30 bg-brand-soft/60 px-6 py-5 text-center">
        <div className="mx-auto flex size-9 items-center justify-center rounded-full bg-brand text-white">
          <CheckIcon />
        </div>
        <p className="mt-3 text-[15px] font-medium text-zinc-900">
          {alreadyJoined ? "You're already on the list." : hero.successMessage}
        </p>
        <p className="mt-1 text-[13px] text-zinc-500">
          Watch the counter below — it just ticked up for everyone.
        </p>
      </div>
    );
  }

  return (
    <form onSubmit={onSubmit} className="w-full max-w-md">
      <div className="flex flex-col gap-2 sm:flex-row">
        <input
          type="email"
          inputMode="email"
          autoComplete="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          placeholder={hero.emailPlaceholder}
          aria-label="Email address"
          required
          className="h-11 flex-1 rounded-full border border-zinc-300 bg-white px-4 text-[15px] text-zinc-900 outline-none transition placeholder:text-zinc-400 focus:border-brand focus:ring-2 focus:ring-brand/20"
        />
        <button
          type="submit"
          disabled={status === "sending"}
          className="inline-flex h-11 items-center justify-center rounded-full bg-zinc-900 px-6 text-[15px] font-medium text-white transition-colors hover:bg-zinc-700 disabled:opacity-60"
        >
          {status === "sending" ? "Joining…" : hero.ctaLabel}
        </button>
      </div>
      {error ? (
        <p className="mt-2 text-[13px] text-red-600">{error}</p>
      ) : (
        <p className="mt-2 text-[13px] text-zinc-400">
          No spam. One invite, the day we launch.
        </p>
      )}
    </form>
  );
}

/* ----------------------------- live counter ---------------------------- */

interface WaitlistStatRow {
  id: string;
  count: number;
  updatedAt: string;
}

function LiveCounter({ seed, label }: { seed: number; label: string }) {
  return (
    <EnsureGuest fallback={<CounterView value={seed} label={label} />}>
      <LiveCounterInner seed={seed} label={label} />
    </EnsureGuest>
  );
}

function LiveCounterInner({ seed, label }: { seed: number; label: string }) {
  // Live entity query over the public WaitlistStat row. `db.useQuery` re-renders
  // the instant the row changes — in THIS tab and every other open tab, because
  // the change syncs through the shared replica. `loading` is true until the
  // first sync settles; until then we show the seed (never a flash of 0).
  const { data, loading } = db.useQuery<WaitlistStatRow>("WaitlistStat");
  const real = data.length > 0 ? data[0].count : 0;
  const value = seed + real;
  return <CounterView value={value} label={label} live={!loading} />;
}

function CounterView({
  value,
  label,
  live,
}: {
  value: number;
  label: string;
  live?: boolean;
}) {
  const shown = useCountUp(value);
  return (
    <div className="inline-flex items-center gap-2.5 rounded-full border border-zinc-200 bg-white px-4 py-2 shadow-sm">
      {live ? (
        <span className="relative flex size-2">
          <span className="absolute inline-flex size-2 animate-ping rounded-full bg-brand/70" />
          <span className="relative inline-flex size-2 rounded-full bg-brand" />
        </span>
      ) : (
        <span className="inline-flex size-2 rounded-full bg-zinc-300" />
      )}
      <span className="text-[15px] font-semibold tabular-nums text-zinc-900">
        {shown.toLocaleString()}
      </span>
      <span className="text-[13px] text-zinc-500">{label}</span>
    </div>
  );
}

// Smoothly tween the displayed number toward `target` whenever it changes, so a
// live increment animates up instead of snapping. First paint shows the value
// directly (no animation).
function useCountUp(target: number): number {
  const [shown, setShown] = useState(target);
  const fromRef = useRef(target);
  const rafRef = useRef<number | null>(null);

  useEffect(() => {
    const from = fromRef.current;
    if (from === target) return;
    const start = performance.now();
    const duration = 600;
    const step = (now: number) => {
      const t = Math.min(1, (now - start) / duration);
      // easeOutCubic
      const eased = 1 - Math.pow(1 - t, 3);
      const next = Math.round(from + (target - from) * eased);
      setShown(next);
      if (t < 1) {
        rafRef.current = requestAnimationFrame(step);
      } else {
        fromRef.current = target;
      }
    };
    rafRef.current = requestAnimationFrame(step);
    return () => {
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
      fromRef.current = target;
    };
  }, [target]);

  return shown;
}

function CheckIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M20 6 9 17l-5-5" />
    </svg>
  );
}
