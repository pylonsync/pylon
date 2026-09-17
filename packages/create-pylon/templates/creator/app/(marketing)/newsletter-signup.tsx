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
      <div className="mt-3">
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
          : "Something went wrong. Try again in a moment.",
      );
      setStatus("idle");
    }
  }

  if (status === "done") {
    return (
      <p className="border-l-2 border-brand pl-4 text-[17.5px] leading-[1.65] text-ink">
        {alreadyJoined ? "You are already subscribed." : newsletter.successMessage}
      </p>
    );
  }

  return (
    <form onSubmit={onSubmit} className="max-w-md">
      <div className="flex gap-2">
        <input
          type="email"
          inputMode="email"
          autoComplete="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          placeholder={newsletter.emailPlaceholder}
          aria-label="Email address"
          required
          className="h-11 min-w-0 flex-1 rounded-md border border-rule bg-white px-3.5 text-[16px] text-ink outline-none transition placeholder:text-ink-2/60 focus:border-brand focus:ring-2 focus:ring-brand/20"
        />
        <button
          type="submit"
          disabled={status === "sending"}
          className="inline-flex h-11 shrink-0 items-center justify-center rounded-md bg-brand px-5 text-[16px] font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-60"
        >
          {status === "sending" ? "Subscribing…" : newsletter.ctaLabel}
        </button>
      </div>
      {error ? (
        <p className="mt-2 text-[15px] text-red-700">{error}</p>
      ) : null}
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

// Plain text under the form. The fallback shows the seed until the guest
// session connects; `live` is set once the count is synced.
function CounterView({ value, label, live }: { value: number; label: string; live?: boolean }) {
  const shown = useCountUp(value);
  return (
    <p className="text-[15px] text-ink-2" data-live={live ? "true" : undefined}>
      <span className="tabular-nums text-ink">{shown.toLocaleString()}</span> {label}
    </p>
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
