"use client";

import React, { useEffect, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { EnsureGuest } from "@pylonsync/client";
import { siteConfig } from "@/lib/site.config";

// The realtime pieces of the page, both driven by the public, PII-free Capacity
// row via `db.useQuery("Capacity")` — so when the owner books a project from the
// dashboard, the "N slots open" number drops live in every open tab. No refresh.
//
//   • <LiveSlots>   — the hero pill ("3 project slots open · Q3 2026").
//   • <ContactForm> — the "start a project" form. submitInquiry is a public
//                     mutation, so it works for anonymous visitors; the Inquiry
//                     it writes is pure PII and can never be read back by a
//                     client. The form only ever reads the slot count.
//
// Both are wrapped in <EnsureGuest>, which mints an anonymous guest session so
// the sync connection (which powers the live count) is established. That session
// holds no PII and can't read the Inquiry table.

interface CapacityRow {
  id: string;
  label: string;
  openSlots: number;
  updatedAt: string;
}

function useSeedCapacity() {
  // Create the Capacity row from config on first visit (idempotent server-side).
  useEffect(() => {
    void callFn("seedCapacity", {});
  }, []);
}

/* ------------------------------ hero pill ------------------------------ */

export function LiveSlots() {
  return (
    <EnsureGuest fallback={<SlotsPill loading />}>
      <LiveSlotsInner />
    </EnsureGuest>
  );
}

function LiveSlotsInner() {
  useSeedCapacity();
  const { data, loading } = db.useQuery<CapacityRow>("Capacity");
  const row = data[0];
  return <SlotsPill openSlots={row?.openSlots} label={row?.label} live={!loading} />;
}

function SlotsPill({
  openSlots,
  label,
  live,
  loading,
}: {
  openSlots?: number;
  label?: string;
  live?: boolean;
  loading?: boolean;
}) {
  const open = openSlots ?? siteConfig.capacity.openSlots;
  const period = label || siteConfig.capacity.label;
  const bookedOut = open <= 0;
  return (
    <span className="inline-flex items-center gap-2 rounded-full border border-zinc-200 bg-white px-3.5 py-1.5 text-[13px] font-medium text-zinc-700 shadow-sm">
      {loading ? (
        <span className="inline-flex size-2 rounded-full bg-zinc-300" />
      ) : (
        <span className="relative flex size-2">
          {live && !bookedOut ? (
            <span className="absolute inline-flex size-2 animate-ping rounded-full bg-green-500/60" />
          ) : null}
          <span
            className={"relative inline-flex size-2 rounded-full " + (bookedOut ? "bg-zinc-400" : "bg-green-600")}
          />
        </span>
      )}
      {bookedOut ? (
        <span>Booked through {period}</span>
      ) : (
        <span>
          <span className="tabular-nums text-zinc-900">{open}</span>{" "}
          {open === 1 ? "project slot" : "project slots"} open · {period}
        </span>
      )}
    </span>
  );
}

/* ----------------------------- contact form ---------------------------- */

export function ContactForm() {
  return (
    <EnsureGuest fallback={<FormShell disabled />}>
      <ContactFormInner />
    </EnsureGuest>
  );
}

function ContactFormInner() {
  useSeedCapacity();
  const { data } = db.useQuery<CapacityRow>("Capacity");
  const row = data[0];
  const bookedOut = (row?.openSlots ?? siteConfig.capacity.openSlots) <= 0;
  return <FormShell bookedOut={bookedOut} period={row?.label} />;
}

function FormShell({
  disabled,
  bookedOut,
  period,
}: {
  disabled?: boolean;
  bookedOut?: boolean;
  period?: string;
}) {
  const { contact } = siteConfig;
  const [form, setForm] = useState({
    name: "",
    email: "",
    company: "",
    projectType: "",
    budget: "",
    message: "",
  });
  const [status, setStatus] = useState<"idle" | "sending" | "done">("idle");
  const [error, setError] = useState<string | null>(null);

  const set = (k: keyof typeof form) => (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement>) =>
    setForm((f) => ({ ...f, [k]: e.target.value }));

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (status === "sending") return;
    if (!form.name.trim() || !form.email.trim()) {
      setError("Your name and email are required.");
      return;
    }
    setStatus("sending");
    setError(null);
    try {
      await callFn<{ ok: boolean }>("submitInquiry", {
        name: form.name.trim(),
        email: form.email.trim(),
        company: form.company.trim() || undefined,
        projectType: form.projectType || undefined,
        budget: form.budget || undefined,
        message: form.message.trim() || undefined,
      });
      setStatus("done");
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(
        /valid email|INVALID_ARGS/i.test(msg)
          ? "Please enter a valid email address."
          : "Something went wrong — try again in a moment.",
      );
      setStatus("idle");
    }
  }

  if (status === "done") {
    return (
      <div className="rounded-2xl border border-brand/30 bg-brand-soft/50 px-6 py-8 text-center">
        <div className="mx-auto flex size-10 items-center justify-center rounded-full bg-brand text-white">
          <CheckIcon />
        </div>
        <p className="mt-3 text-[15px] font-semibold text-zinc-900">{contact.confirmationMessage}</p>
      </div>
    );
  }

  return (
    <form onSubmit={onSubmit} className="rounded-2xl border border-zinc-200 bg-white p-6 shadow-sm sm:p-7">
      {bookedOut ? (
        <p className="mb-4 rounded-lg bg-amber-50 px-3 py-2 text-[13px] text-amber-700">
          We&apos;re fully booked{period ? ` through ${period}` : ""} — send a note anyway and we&apos;ll reach out when a slot opens.
        </p>
      ) : null}
      <div className="grid gap-3 sm:grid-cols-2">
        <Field label="Name" required>
          <input value={form.name} onChange={set("name")} autoComplete="name" aria-label="Name" className={inputCls} disabled={disabled} />
        </Field>
        <Field label="Email" required>
          <input type="email" value={form.email} onChange={set("email")} autoComplete="email" aria-label="Email" className={inputCls} disabled={disabled} />
        </Field>
        <Field label="Company">
          <input value={form.company} onChange={set("company")} autoComplete="organization" aria-label="Company" className={inputCls} disabled={disabled} />
        </Field>
        <Field label="Project type">
          <select value={form.projectType} onChange={set("projectType")} aria-label="Project type" className={inputCls} disabled={disabled}>
            <option value="">Select…</option>
            {contact.projectTypes.map((t) => (
              <option key={t} value={t}>{t}</option>
            ))}
          </select>
        </Field>
      </div>
      <div className="mt-3">
        <Field label="Budget">
          <select value={form.budget} onChange={set("budget")} aria-label="Budget" className={inputCls} disabled={disabled}>
            <option value="">Select…</option>
            {contact.budgets.map((b) => (
              <option key={b} value={b}>{b}</option>
            ))}
          </select>
        </Field>
      </div>
      <div className="mt-3">
        <Field label="What are you building?">
          <textarea
            value={form.message}
            onChange={set("message")}
            rows={4}
            aria-label="Message"
            placeholder="A sentence or two about the project, timeline, and what success looks like."
            className={inputCls + " resize-none py-2.5"}
            disabled={disabled}
          />
        </Field>
      </div>
      {error ? <p className="mt-3 text-[13px] text-red-600">{error}</p> : null}
      <button
        type="submit"
        disabled={status === "sending" || disabled}
        className="mt-5 inline-flex h-11 w-full items-center justify-center rounded-full bg-brand text-[15px] font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-60 sm:w-auto sm:px-7"
      >
        {status === "sending" ? "Sending…" : "Send inquiry"}
      </button>
    </form>
  );
}

function Field({ label, required, children }: { label: string; required?: boolean; children: React.ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1.5 block text-[12.5px] font-medium text-zinc-600">
        {label}
        {required ? <span className="text-brand"> *</span> : null}
      </span>
      {children}
    </label>
  );
}

const inputCls =
  "h-10 w-full rounded-lg border border-zinc-300 bg-white px-3 text-[14px] text-zinc-900 outline-none transition placeholder:text-zinc-400 focus:border-brand focus:ring-2 focus:ring-brand/20 disabled:opacity-60";

function CheckIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M20 6 9 17l-5-5" />
    </svg>
  );
}
