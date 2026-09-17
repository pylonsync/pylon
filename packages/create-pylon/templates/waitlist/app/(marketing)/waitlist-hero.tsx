"use client";

import React, { useEffect, useRef, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { EnsureGuest } from "@pylonsync/client";
import type { WaitlistConfig } from "@/lib/site.config";

// The two client islands on the landing page: the email-capture form (in the
// hero) and the LIVE signup count (under the facts). The count is a live
// `db.useQuery("WaitlistStat")` over the public, PII-free aggregate row, so
// the moment anyone (this tab or another) submits an email, joinWaitlist
// updates that row and the new count syncs to every open tab through the
// replica. No refresh, no polling.
//
// The signup form (joinWaitlist) is a public mutation, so it works for any
// anonymous visitor. The count needs a live sync connection, so it's wrapped
// in <EnsureGuest>, which mints an anonymous guest session on first load. That
// session holds no PII, and the Signup table stays unreadable to it; the page
// only ever reads the aggregate count, never an email.

/* ----------------------------- signup form ---------------------------- */

export function SignupForm({ hero }: { hero: WaitlistConfig["hero"] }) {
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
      // The live count updates itself via the reactive push.
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(
        /valid email|INVALID_ARGS/i.test(msg)
          ? "Enter a valid email address."
          : "Something went wrong. Try again in a moment.",
      );
      setStatus("idle");
    }
  }

  if (status === "done") {
    return (
      <div className="border-t border-brand pt-4">
        <p className="text-[15px] font-medium text-chalk">
          {alreadyJoined ? "You are already on the list." : hero.successMessage}
        </p>
        <p className="mt-1 font-mono-ui text-[12px] text-chalk-2">
          The count below went up for everyone.
        </p>
      </div>
    );
  }

  return (
    <form onSubmit={onSubmit}>
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
          className="h-11 flex-1 rounded-md border border-line bg-paper px-3.5 text-[15px] text-chalk outline-none transition placeholder:text-chalk-2 focus:border-brand"
        />
        <button
          type="submit"
          disabled={status === "sending"}
          className="inline-flex h-11 shrink-0 items-center justify-center rounded-md bg-brand px-5 text-[15px] font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-60"
        >
          {status === "sending" ? "Joining…" : hero.ctaLabel}
        </button>
      </div>
      <p className={`mt-2.5 font-mono-ui text-[12px] ${error ? "text-red-400" : "text-chalk-2"}`}>
        {error ?? hero.formHint}
      </p>
    </form>
  );
}

/* ----------------------------- live count ---------------------------- */

interface WaitlistStatRow {
  id: string;
  count: number;
  updatedAt: string;
}

export function LiveCount({ seed, label }: { seed: number; label: string }) {
  return (
    <EnsureGuest fallback={<CountView value={seed} label={label} />}>
      <LiveCountInner seed={seed} label={label} />
    </EnsureGuest>
  );
}

function LiveCountInner({ seed, label }: { seed: number; label: string }) {
  // Live entity query over the public WaitlistStat row. `db.useQuery` re-renders
  // the instant the row changes, in THIS tab and every other open tab, because
  // the change syncs through the shared replica. `loading` is true until the
  // first sync settles; until then we show the seed (never a flash of 0).
  const { data, loading } = db.useQuery<WaitlistStatRow>("WaitlistStat");
  const real = data.length > 0 ? data[0].count : 0;
  const value = seed + real;
  return <CountView value={value} label={label} live={!loading} />;
}

function CountView({
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
    <p className="flex items-center gap-3 font-mono-ui text-[14px] text-chalk-2">
      <span
        aria-hidden
        className={live ? "size-2 rounded-full bg-brand" : "size-2 rounded-full bg-line"}
      />
      <span className="tabular-nums text-chalk">{shown.toLocaleString()}</span>
      <span>{label}</span>
      <span className="text-[12px]">{live ? "live" : ""}</span>
    </p>
  );
}

// Tween the displayed number toward `target` whenever it changes, so a live
// increment counts up instead of snapping. First paint shows the value directly.
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
