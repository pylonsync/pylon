"use client";

import React, { useEffect, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { EnsureGuest } from "@pylonsync/client";
import { siteConfig } from "@/lib/site.config";

// The realtime pieces of the page, both driven by the public, PII-free Capacity
// row via `db.useQuery("Capacity")` — so when the owner books a project from the
// dashboard, the "N slots open" number drops live in every open tab. No refresh.
//
//   • <LiveSlots>   — the hero line ("3 project slots open, Q3 2026").
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

/* ------------------------------ hero line ------------------------------ */

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
    <span className="inline-flex items-center gap-2.5 text-[14px] text-zinc-700">
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
          <span className="tabular-nums text-ink">{open}</span>{" "}
          {open === 1 ? "project slot" : "project slots"} open, {period}
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
          : "Something went wrong. Try again in a moment.",
      );
      setStatus("idle");
    }
  }

  if (status === "done") {
    return (
      <div className="border-t border-ink pt-6">
        <p className="font-display text-[1.75rem] leading-tight text-ink">Sent.</p>
        <p className="mt-2 text-[15px] leading-relaxed text-zinc-700">{contact.confirmationMessage}</p>
      </div>
    );
  }

  return (
    <form onSubmit={onSubmit}>
      {bookedOut ? (
        <p className="mb-5 border-l-2 border-ink pl-3 text-[14px] text-zinc-700">
          We are fully booked{period ? ` through ${period}` : ""}. Send a note anyway and we will write when a slot opens.
        </p>
      ) : null}
      <div className="grid gap-5 sm:grid-cols-2">
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
      <div className="mt-5">
        <Field label="Budget">
          <select value={form.budget} onChange={set("budget")} aria-label="Budget" className={inputCls} disabled={disabled}>
            <option value="">Select…</option>
            {contact.budgets.map((b) => (
              <option key={b} value={b}>{b}</option>
            ))}
          </select>
        </Field>
      </div>
      <div className="mt-5">
        <Field label="What are you building?">
          <textarea
            value={form.message}
            onChange={set("message")}
            rows={4}
            aria-label="Message"
            placeholder="A sentence or two about the project and the timeline."
            className={inputCls + " h-auto resize-none py-2.5"}
            disabled={disabled}
          />
        </Field>
      </div>
      {error ? <p className="mt-4 text-[14px] text-red-700">{error}</p> : null}
      <button
        type="submit"
        disabled={status === "sending" || disabled}
        className="mt-6 inline-flex h-11 w-full items-center justify-center bg-ink px-7 text-[15px] font-medium text-white transition-colors hover:bg-zinc-700 disabled:opacity-60 sm:w-auto"
      >
        {status === "sending" ? "Sending…" : "Send inquiry"}
      </button>
    </form>
  );
}

function Field({ label, required, children }: { label: string; required?: boolean; children: React.ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1.5 block text-[11px] font-medium uppercase tracking-[0.08em] text-zinc-600">
        {label}
        {required ? <span className="text-zinc-500"> *</span> : null}
      </span>
      {children}
    </label>
  );
}

const inputCls =
  "h-11 w-full rounded-none border-0 border-b border-zinc-400 bg-transparent px-0 text-[15px] text-ink outline-none transition-colors placeholder:text-zinc-400 focus:border-ink disabled:opacity-60";
