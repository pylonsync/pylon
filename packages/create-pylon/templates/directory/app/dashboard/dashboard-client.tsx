"use client";

import React, { useEffect, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { useAuth } from "@pylonsync/client";
import { parseTags, type SubmissionRow, type OwnerSubmissionsResult } from "@/lib/directory";

// The curator's moderation queue. Liveness rides the public Listing table:
// `db.useQuery("Listing")` re-renders when listings change (approving a
// submission creates one), and that same signal re-fetches the owner-gated
// `submissionsForOwner` — so the queue stays current without a reload, while
// submitter PII only ever travels through the gated call.
export function DirectoryDashboard({ userEmail }: { userEmail: string }) {
  const { data: listings } = db.useQuery<{ id: string }>("Listing");
  const liveCount = listings.length;

  const [submissions, setSubmissions] = useState<SubmissionRow[] | null>(null);
  const [denied, setDenied] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  async function load() {
    try {
      const r = await callFn<OwnerSubmissionsResult>("submissionsForOwner", {});
      if (!r.authorized) setDenied(true);
      else {
        setSubmissions(r.submissions);
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
  }, [liveCount]);

  async function act(id: string, fn: "approveSubmission" | "rejectSubmission") {
    setBusyId(id);
    try {
      await callFn(fn, { submissionId: id });
      await load();
    } finally {
      setBusyId(null);
    }
  }

  if (denied) return <OwnerOnly email={userEmail} />;
  if (error) {
    return <div className="rounded-xl border border-red-200 bg-red-50 px-5 py-4 text-sm text-red-700">{error}</div>;
  }
  if (!submissions) return <Skeleton />;

  const pending = submissions.filter((s) => s.status === "new");
  const approved = submissions.filter((s) => s.status === "approved").length;

  return (
    <div className="space-y-8">
      <div>
        <h1 className="text-xl font-semibold tracking-tight">Submissions</h1>
        <p className="mt-1 text-sm text-zinc-500">
          Live — new submissions land here; approving one publishes it to the directory instantly.
        </p>
      </div>

      <div className="grid gap-4 sm:grid-cols-3">
        <Stat label="Pending" value={String(pending.length)} />
        <Stat label="Approved" value={String(approved)} />
        <Stat label="Live listings" value={String(liveCount)} />
      </div>

      <div className="rounded-xl border border-zinc-200 bg-white">
        <div className="border-b border-zinc-100 px-4 py-3 text-sm font-semibold text-zinc-900">
          Queue <span className="font-normal text-zinc-400">({submissions.length})</span>
        </div>
        {submissions.length === 0 ? (
          <p className="p-8 text-center text-sm text-zinc-500">No submissions yet.</p>
        ) : (
          <ul className="divide-y divide-zinc-100">
            {submissions.map((s) => (
              <li key={s.id} className="px-4 py-3.5">
                <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-2">
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <a href={s.url} target="_blank" rel="noopener noreferrer" className="text-[14px] font-medium text-zinc-900 hover:text-brand">
                        {s.name}
                      </a>
                      <span className="rounded-full bg-zinc-100 px-2 py-0.5 text-[11px] font-medium text-zinc-500">{s.category}</span>
                      <StatusBadge status={s.status} />
                    </div>
                    {s.tagline ? <p className="mt-1 text-[13.5px] text-zinc-600">{s.tagline}</p> : null}
                    <div className="mt-1 truncate text-[12px] text-zinc-400">
                      {s.submitterName} · {s.submitterEmail}
                      {parseTags(s.tags).length ? ` · ${parseTags(s.tags).map((t) => "#" + t).join(" ")}` : ""}
                    </div>
                  </div>
                  {s.status === "new" ? (
                    <div className="flex items-center gap-2">
                      <button
                        type="button"
                        disabled={busyId === s.id}
                        onClick={() => act(s.id, "approveSubmission")}
                        className="rounded-md bg-brand px-3 py-1.5 text-[12.5px] font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-50"
                      >
                        {busyId === s.id ? "…" : "Approve"}
                      </button>
                      <button
                        type="button"
                        disabled={busyId === s.id}
                        onClick={() => act(s.id, "rejectSubmission")}
                        className="rounded-md border border-zinc-300 px-3 py-1.5 text-[12.5px] font-medium text-zinc-600 transition-colors hover:border-red-300 hover:text-red-600 disabled:opacity-50"
                      >
                        Reject
                      </button>
                    </div>
                  ) : null}
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-xl border border-zinc-200 bg-white p-4">
      <div className="text-[11px] font-medium uppercase tracking-wide text-zinc-400">{label}</div>
      <div className="mt-1 text-2xl font-semibold tabular-nums text-zinc-900">{value}</div>
    </div>
  );
}

function StatusBadge({ status }: { status: string }) {
  const tone =
    status === "approved"
      ? "bg-green-50 text-green-700"
      : status === "rejected"
        ? "bg-zinc-100 text-zinc-400"
        : "bg-amber-50 text-amber-700"; // new
  const label = status === "new" ? "pending" : status;
  return <span className={"rounded-full px-2 py-0.5 text-[10px] font-medium capitalize " + tone}>{label}</span>;
}

function OwnerOnly({ email }: { email: string }) {
  return (
    <div className="rounded-xl border border-dashed border-zinc-300 px-6 py-12 text-center">
      <h1 className="text-lg font-semibold">This dashboard is owner-only</h1>
      <p className="mx-auto mt-2 max-w-md text-sm text-zinc-500">
        You&apos;re signed in as <span className="font-medium text-zinc-700">{email || "this account"}</span>.
        Only the curator can review submissions. Set{" "}
        <code className="rounded bg-zinc-100 px-1.5 py-0.5 text-[12px]">PYLON_OWNER_EMAIL={email || "you@yourdirectory.com"}</code>{" "}
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
      <div className="h-6 w-32 animate-pulse rounded bg-zinc-100" />
      <div className="grid gap-4 sm:grid-cols-3">
        {[0, 1, 2].map((i) => (
          <div key={i} className="h-20 animate-pulse rounded-xl bg-zinc-100" />
        ))}
      </div>
      <div className="h-48 animate-pulse rounded-xl bg-zinc-100" />
    </div>
  );
}
