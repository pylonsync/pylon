"use client";

import React, { useEffect, useRef, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { EnsureGuest } from "@pylonsync/client";
import type { CreatorConfig } from "@/lib/site.config";

// The newsletter signup — email capture plus a LIVE subscriber counter. This is
// the realtime proof: the counter is a live `db.useQuery("SubscriberCount")`
// over the public, PII-free aggregate row, so the moment anyone (this tab or
// another) subscribes, `subscribe` updates that row and the new count syncs to
// every open tab. No refresh.
//
// The form (subscribe) is a public mutation, so it works for any anonymous
// visitor. The counter needs a live sync connection, so it's wrapped in
// <EnsureGuest>, which mints an anonymous guest session. It holds no PII, and
// the Subscriber table stays unreadable to it — only the aggregate count leaves.

type Props = { newsletter: CreatorConfig["newsletter"] };

export function NewsletterSignup({ newsletter }: Props) {
  return (
    <div>
      <SubscribeForm newsletter={newsletter} />
      <div className="mt-5">
        <LiveCounter seed={newsletter.seedCount ?? 0} label={newsletter.counterLabel} />
      </div>
    </div>
  );
}

function SubscribeForm({ newsletter }: Props) {
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
      const res = await callFn<{ ok: boolean; alreadyJoined: boolean }>("subscribe", {
        email: value,
      });
      setAlreadyJoined(res.alreadyJoined);
      setStatus("done");
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
      <div className="rounded-xl border border-brand/30 bg-brand-soft/50 px-5 py-4">
        <p className="text-[15px] font-medium text-zinc-900">
          {alreadyJoined ? "You're already subscribed." : newsletter.successMessage}
        </p>
      </div>
    );
  }

  return (
    <form onSubmit={onSubmit} className="max-w-md">
      <div className="flex flex-col gap-2 sm:flex-row">
        <input
          type="email"
          inputMode="email"
          autoComplete="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          placeholder={newsletter.emailPlaceholder}
          aria-label="Email address"
          required
          className="h-11 flex-1 rounded-full border border-zinc-300 bg-white px-4 text-[15px] text-zinc-900 outline-none transition placeholder:text-zinc-400 focus:border-brand focus:ring-2 focus:ring-brand/20"
        />
        <button
          type="submit"
          disabled={status === "sending"}
          className="inline-flex h-11 items-center justify-center rounded-full bg-brand px-6 text-[15px] font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-60"
        >
          {status === "sending" ? "Subscribing…" : newsletter.ctaLabel}
        </button>
      </div>
      {error ? (
        <p className="mt-2 text-[13px] text-red-600">{error}</p>
      ) : (
        <p className="mt-2 text-[13px] text-zinc-400">One email a week. Unsubscribe anytime.</p>
      )}
    </form>
  );
}

interface SubscriberCountRow {
  id: string;
  count: number;
}

function LiveCounter({ seed, label }: { seed: number; label: string }) {
  return (
    <EnsureGuest fallback={<CounterView value={seed} label={label} />}>
      <LiveCounterInner seed={seed} label={label} />
    </EnsureGuest>
  );
}

function LiveCounterInner({ seed, label }: { seed: number; label: string }) {
  const { data, loading } = db.useQuery<SubscriberCountRow>("SubscriberCount");
  const real = data.length > 0 ? data[0].count : 0;
  return <CounterView value={seed + real} label={label} live={!loading} />;
}

function CounterView({ value, label, live }: { value: number; label: string; live?: boolean }) {
  const shown = useCountUp(value);
  return (
    <div className="inline-flex items-center gap-2.5 text-[14px] text-zinc-500">
      {live ? (
        <span className="relative flex size-2">
          <span className="absolute inline-flex size-2 animate-ping rounded-full bg-brand/70" />
          <span className="relative inline-flex size-2 rounded-full bg-brand" />
        </span>
      ) : (
        <span className="inline-flex size-2 rounded-full bg-zinc-300" />
      )}
      <span className="font-semibold tabular-nums text-zinc-900">{shown.toLocaleString()}</span>
      {label}
    </div>
  );
}

function useCountUp(target: number): number {
  const [shown, setShown] = useState(target);
  const fromRef = useRef(target);
  const rafRef = useRef<number | null>(null);
  useEffect(() => {
    const from = fromRef.current;
    if (from === target) return;
    const start = performance.now();
    const dur = 600;
    const step = (now: number) => {
      const t = Math.min(1, (now - start) / dur);
      const eased = 1 - Math.pow(1 - t, 3);
      setShown(Math.round(from + (target - from) * eased));
      if (t < 1) rafRef.current = requestAnimationFrame(step);
      else fromRef.current = target;
    };
    rafRef.current = requestAnimationFrame(step);
    return () => {
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
      fromRef.current = target;
    };
  }, [target]);
  return shown;
}
