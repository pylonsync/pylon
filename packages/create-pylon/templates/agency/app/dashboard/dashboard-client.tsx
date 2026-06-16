"use client";

import React, { useEffect, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { useAuth } from "@pylonsync/client";
import type { InquiryRow, OwnerInquiriesResult } from "@/lib/agency";

interface CapacityRow {
  id: string;
  label: string;
  openSlots: number;
  updatedAt: string;
}

// The owner's live pipeline. Liveness rides the SAME public Capacity row the
// landing page uses: `db.useQuery("Capacity")` re-renders the instant slots
// change (cross-tab, via the replica). The leads themselves never sync — they
// come from the owner-gated `inquiriesForOwner`, (re)fetched on mount and
// whenever capacity changes (which is exactly when a lead is booked/released).
// So the pipeline stays live, but PII only ever travels through the gated call.
export function AgencyDashboard({ userEmail }: { userEmail: string }) {
  const { data: caps } = db.useQuery<CapacityRow>("Capacity");
  const cap = caps[0];
  const liveKey = `${cap?.openSlots ?? "?"}:${cap?.label ?? ""}`;

  const [inquiries, setInquiries] = useState<InquiryRow[] | null>(null);
  const [denied, setDenied] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  async function load() {
    try {
      const r = await callFn<OwnerInquiriesResult>("inquiriesForOwner", {});
      if (!r.authorized) setDenied(true);
      else {
        setInquiries(r.inquiries);
        setDenied(false);
        setError(null);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [liveKey]);

  async function act(id: string, fn: "bookInquiry" | "declineInquiry") {
    setBusyId(id);
    try {
      await callFn(fn, { inquiryId: id });
      await load();
    } finally {
      setBusyId(null);
    }
  }

  if (denied) return <OwnerOnly email={userEmail} />;
  if (error) {
    return <div className="rounded-xl border border-red-200 bg-red-50 px-5 py-4 text-sm text-red-700">{error}</div>;
  }
  if (!inquiries) return <Skeleton />;

  const newCount = inquiries.filter((i) => i.status === "new").length;
  const bookedCount = inquiries.filter((i) => i.status === "booked").length;
  const active = inquiries.filter((i) => i.status !== "declined");

  return (
    <div className="space-y-8">
      <div>
        <h1 className="text-xl font-semibold tracking-tight">Pipeline</h1>
        <p className="mt-1 text-sm text-zinc-500">
          Live — leads land here the moment they&apos;re sent. Booking one drops the open-slot count
          on your site instantly.
        </p>
      </div>

      <div className="grid gap-4 sm:grid-cols-3">
        <Stat label="Open slots" value={String(cap?.openSlots ?? 0)} hint={cap?.label} />
        <Stat label="New leads" value={String(newCount)} />
        <Stat label="Booked" value={String(bookedCount)} />
      </div>

      <CapacityCard cap={cap} />

      {/* Inquiries */}
      <div className="rounded-xl border border-zinc-200 bg-white">
        <div className="border-b border-zinc-100 px-4 py-3 text-sm font-semibold text-zinc-900">
          Inquiries <span className="font-normal text-zinc-400">({active.length})</span>
        </div>
        {inquiries.length === 0 ? (
          <p className="p-8 text-center text-sm text-zinc-500">No inquiries yet — share your site.</p>
        ) : (
          <ul className="divide-y divide-zinc-100">
            {inquiries.map((i) => (
              <li key={i.id} className="px-4 py-3.5">
                <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-2">
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-[14px] font-medium text-zinc-900">{i.name}</span>
                      <StatusBadge status={i.status} />
                    </div>
                    <div className="mt-0.5 truncate text-[12.5px] text-zinc-500">
                      {i.email}
                      {i.company ? ` · ${i.company}` : ""}
                      {i.projectType ? ` · ${i.projectType}` : ""}
                      {i.budget ? ` · ${i.budget}` : ""}
                    </div>
                    {i.message ? (
                      <p className="mt-1.5 line-clamp-2 text-[13px] leading-relaxed text-zinc-600">{i.message}</p>
                    ) : null}
                  </div>
                  <div className="flex items-center gap-2">
                    {i.status !== "booked" ? (
                      <button
                        type="button"
                        disabled={busyId === i.id}
                        onClick={() => act(i.id, "bookInquiry")}
                        className="rounded-md bg-brand px-3 py-1.5 text-[12.5px] font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-50"
                      >
                        {busyId === i.id ? "…" : "Book"}
                      </button>
                    ) : null}
                    {i.status !== "declined" ? (
                      <button
                        type="button"
                        disabled={busyId === i.id}
                        onClick={() => act(i.id, "declineInquiry")}
                        className="rounded-md border border-zinc-300 px-3 py-1.5 text-[12.5px] font-medium text-zinc-600 transition-colors hover:border-red-300 hover:text-red-600 disabled:opacity-50"
                      >
                        {i.status === "booked" ? "Release" : "Decline"}
                      </button>
                    ) : null}
                  </div>
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

// Editable capacity — the number the public hero shows live. Saving calls
// setCapacity; the change syncs straight to every open landing page.
function CapacityCard({ cap }: { cap?: CapacityRow }) {
  const [label, setLabel] = useState(cap?.label ?? "");
  const [slots, setSlots] = useState(String(cap?.openSlots ?? 0));
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  // Keep inputs in sync if the live row changes underneath us (e.g. a booking).
  useEffect(() => {
    setLabel(cap?.label ?? "");
    setSlots(String(cap?.openSlots ?? 0));
  }, [cap?.label, cap?.openSlots]);

  async function save(e: React.FormEvent) {
    e.preventDefault();
    setSaving(true);
    setSaved(false);
    try {
      await callFn("setCapacity", { label: label.trim(), openSlots: Math.max(0, parseInt(slots, 10) || 0) });
      setSaved(true);
    } finally {
      setSaving(false);
    }
  }

  return (
    <form onSubmit={save} className="rounded-xl border border-zinc-200 bg-white p-4">
      <div className="text-sm font-semibold text-zinc-900">Availability</div>
      <p className="mt-1 text-[13px] text-zinc-500">Shown live on your site as “N project slots open”.</p>
      <div className="mt-4 flex flex-wrap items-end gap-3">
        <label className="block">
          <span className="mb-1 block text-[12px] font-medium text-zinc-600">Booking window</span>
          <input
            value={label}
            onChange={(e) => { setLabel(e.target.value); setSaved(false); }}
            placeholder="Q3 2026"
            className="h-9 w-40 rounded-lg border border-zinc-300 px-3 text-[14px] outline-none focus:border-brand focus:ring-2 focus:ring-brand/20"
          />
        </label>
        <label className="block">
          <span className="mb-1 block text-[12px] font-medium text-zinc-600">Open slots</span>
          <input
            type="number"
            min={0}
            value={slots}
            onChange={(e) => { setSlots(e.target.value); setSaved(false); }}
            className="h-9 w-24 rounded-lg border border-zinc-300 px-3 text-[14px] tabular-nums outline-none focus:border-brand focus:ring-2 focus:ring-brand/20"
          />
        </label>
        <button
          type="submit"
          disabled={saving}
          className="h-9 rounded-lg bg-zinc-900 px-4 text-[13px] font-medium text-white transition-colors hover:bg-zinc-700 disabled:opacity-50"
        >
          {saving ? "Saving…" : saved ? "Saved ✓" : "Save"}
        </button>
      </div>
    </form>
  );
}

function Stat({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="rounded-xl border border-zinc-200 bg-white p-4">
      <div className="text-[11px] font-medium uppercase tracking-wide text-zinc-400">{label}</div>
      <div className="mt-1 text-2xl font-semibold tabular-nums text-zinc-900">{value}</div>
      {hint ? <div className="mt-0.5 text-[12px] text-zinc-400">{hint}</div> : null}
    </div>
  );
}

function StatusBadge({ status }: { status: string }) {
  const tone =
    status === "booked"
      ? "bg-green-50 text-green-700"
      : status === "declined"
        ? "bg-zinc-100 text-zinc-400"
        : "bg-amber-50 text-amber-700"; // new
  const label = status === "new" ? "new lead" : status;
  return <span className={"rounded-full px-2 py-0.5 text-[10px] font-medium capitalize " + tone}>{label}</span>;
}

function OwnerOnly({ email }: { email: string }) {
  return (
    <div className="rounded-xl border border-dashed border-zinc-300 px-6 py-12 text-center">
      <h1 className="text-lg font-semibold">This dashboard is owner-only</h1>
      <p className="mx-auto mt-2 max-w-md text-sm text-zinc-500">
        You&apos;re signed in as <span className="font-medium text-zinc-700">{email || "this account"}</span>.
        Only the studio owner can see inquiries. Set{" "}
        <code className="rounded bg-zinc-100 px-1.5 py-0.5 text-[12px]">PYLON_OWNER_EMAIL={email || "you@studio.com"}</code>{" "}
        in your <code className="rounded bg-zinc-100 px-1.5 py-0.5 text-[12px]">.env</code>, restart, and reload —
        or sign in with the owner account.
      </p>
    </div>
  );
}

export function UserMenu({ email }: { email: string }) {
  const { signOut } = useAuth();
  const initial = (email.trim()[0] || "?").toUpperCase();
  async function onSignOut() {
    await signOut();
    window.location.assign("/");
  }
  return (
    <details className="group relative">
      <summary className="flex size-8 cursor-pointer select-none list-none items-center justify-center rounded-full bg-zinc-900 text-[12px] font-semibold text-white marker:hidden [&::-webkit-details-marker]:hidden">
        {initial}
      </summary>
      <div className="absolute right-0 top-full z-40 mt-2 w-56 overflow-hidden rounded-xl border border-zinc-200 bg-white py-1 shadow-[0_16px_48px_-16px_rgba(0,0,0,0.25)]">
        <div className="border-b border-zinc-100 px-3 py-2">
          <div className="truncate text-[13px] font-medium text-zinc-900">{email || "Signed in"}</div>
        </div>
        <button
          type="button"
          onClick={onSignOut}
          className="flex w-full items-center px-3 py-2 text-left text-[13px] text-zinc-700 transition-colors hover:bg-zinc-50"
        >
          Sign out
        </button>
      </div>
    </details>
  );
}

function Skeleton() {
  return (
    <div className="space-y-8">
      <div className="h-6 w-28 animate-pulse rounded bg-zinc-100" />
      <div className="grid gap-4 sm:grid-cols-3">
        {[0, 1, 2].map((i) => (
          <div key={i} className="h-20 animate-pulse rounded-xl bg-zinc-100" />
        ))}
      </div>
      <div className="h-28 animate-pulse rounded-xl bg-zinc-100" />
      <div className="h-48 animate-pulse rounded-xl bg-zinc-100" />
    </div>
  );
}
