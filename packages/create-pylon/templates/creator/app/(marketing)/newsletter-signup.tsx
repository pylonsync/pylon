"use client";

import React, { useEffect, useRef, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { EnsureGuest } from "@pylonsync/client";
import type { CreatorConfig } from "@/lib/site.config";
import { publicCount } from "@/lib/stats";

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
        <LiveCounter
          importedCount={newsletter.importedCount}
          label={newsletter.counterLabel}
          labelOne={newsletter.counterLabelOne}
        />
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

type CounterProps = { importedCount: number; label: string; labelOne: string };

export function LiveCounter(props: CounterProps) {
  return (
    <EnsureGuest fallback={null}>
      <LiveCounterInner {...props} />
    </EnsureGuest>
  );
}

function LiveCounterInner({ importedCount, label, labelOne }: CounterProps) {
  const { data, loading } = db.useQuery<SubscriberCountRow>("SubscriberCount");

  // In `pylon dev` an empty list gets demo subscribers (functions/seedDemo.ts).
  // The call goes out only when the synced count is empty, so a live site with
  // subscribers never makes it.
  const empty = !loading && data.length === 0;
  const sent = useRef(false);
  useEffect(() => {
    if (!empty || sent.current) return;
    sent.current = true;
    void callFn("seedDemo", {}).catch(() => {});
  }, [empty]);

  if (loading) return null;
  const value = publicCount(data, importedCount);
  // A new newsletter has no subscribers yet; show nothing rather than "0".
  if (value === 0) return null;
  return <CounterView value={value} label={value === 1 ? labelOne : label} />;
}

// Plain text under the form, shown once the count has synced.
function CounterView({ value, label }: { value: number; label: string }) {
  const shown = useCountUp(value);
  return (
    <p className="text-[15px] text-ink-2" data-testid="live-count">
      <span className="tabular-nums text-ink">{shown.toLocaleString("en-US")}</span> {label}
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
