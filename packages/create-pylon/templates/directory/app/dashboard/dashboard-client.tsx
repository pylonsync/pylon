"use client";

import React, { useEffect, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { useAuth } from "@pylonsync/client";
import { ArrowUpRight, Check, LogOut, X } from "lucide-react";
import {
  monogram,
  parseTags,
  type SubmissionRow,
  type OwnerSubmissionsResult,
} from "@/lib/directory";

// The curator's moderation queue. Liveness rides the public Listing table:
// `db.useQuery("Listing")` re-renders when listings change (approving a
// submission creates one), and that same signal re-fetches the owner-gated
// `submissionsForOwner`, so the queue stays current without a reload, while
// submitter PII only ever travels through the gated call.
type Filter = "new" | "approved" | "rejected";

const FILTERS: { id: Filter; label: string }[] = [
  { id: "new", label: "Pending" },
  { id: "approved", label: "Approved" },
  { id: "rejected", label: "Rejected" },
];

export function DirectoryDashboard({ userEmail }: { userEmail: string }) {
  const { data: listings } = db.useQuery<{ id: string }>("Listing");
  const liveCount = listings.length;

  const [submissions, setSubmissions] = useState<SubmissionRow[] | null>(null);
  const [denied, setDenied] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [filter, setFilter] = useState<Filter>("new");
  const [seedDone, setSeedDone] = useState(false);

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

  // In `pylon dev` an empty queue gets demo submissions (functions/seedDemo.ts)
  // before the first load. On a deploy, or for a non-owner, the call writes
  // nothing.
  useEffect(() => {
    void callFn("seedDemo", {})
      .catch(() => {})
      .finally(() => setSeedDone(true));
  }, []);

  useEffect(() => {
    if (!seedDone) return;
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [liveCount, seedDone]);

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
    return <div className="border border-red-200 bg-red-50 px-5 py-4 text-sm text-red-700">{error}</div>;
  }
  if (!submissions) return <Skeleton />;

  const counts: Record<Filter, number> = { new: 0, approved: 0, rejected: 0 };
  for (const s of submissions) {
    if (s.status === "new" || s.status === "approved" || s.status === "rejected") counts[s.status]++;
  }
  const shown = submissions.filter((s) => s.status === filter);

  return (
    <div className="space-y-8">
      <div>
        <h1 className="text-[1.75rem] font-semibold leading-tight tracking-[-0.02em] text-zinc-900 sm:text-[2rem]">
          Review queue
        </h1>
        <p className="mt-2 max-w-xl text-[14px] leading-relaxed text-zinc-600">
          New submissions land here. Approving one publishes it to the directory right away.
        </p>
      </div>

      <dl className="grid grid-cols-3 divide-x divide-zinc-200 border-y border-zinc-200">
        <Stat label="Pending" value={counts.new} accent={counts.new > 0} />
        <Stat label="Approved" value={counts.approved} />
        <Stat label="Live listings" value={liveCount} />
      </dl>

      <section>
        <div className="flex items-center gap-4 border-b border-zinc-200">
          {FILTERS.map((f) => (
            <button
              key={f.id}
              type="button"
              onClick={() => setFilter(f.id)}
              aria-pressed={filter === f.id}
              className={
                "-mb-px border-b-2 pb-2 text-[14px] transition-colors " +
                (filter === f.id
                  ? "border-zinc-900 font-medium text-zinc-900"
                  : "border-transparent text-zinc-500 hover:text-zinc-900")
              }
            >
              {f.label} <span className="font-mono text-[12px] text-zinc-400">{counts[f.id]}</span>
            </button>
          ))}
        </div>

        {shown.length === 0 ? (
          <p className="py-12 text-center text-[14px] text-zinc-500">
            {filter === "new"
              ? "Nothing to review. New submissions appear here as they arrive."
              : `No ${filter} submissions.`}
          </p>
        ) : (
          <ul>
            {shown.map((s) => (
              <QueueItem key={s.id} s={s} busy={busyId === s.id} onAct={act} />
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}

function QueueItem({
  s,
  busy,
  onAct,
}: {
  s: SubmissionRow;
  busy: boolean;
  onAct: (id: string, fn: "approveSubmission" | "rejectSubmission") => void;
}) {
  const m = monogram(s.name);
  const tags = parseTags(s.tags);
  return (
    <li className="grid grid-cols-[36px_minmax(0,1fr)] gap-x-3 gap-y-3 border-b border-zinc-200 py-4 sm:grid-cols-[36px_minmax(0,1fr)_auto]">
      <span
        aria-hidden
        className="flex size-9 items-center justify-center rounded-md font-mono text-[13px] font-semibold"
        style={{ backgroundColor: m.bg, color: m.fg }}
      >
        {m.letters}
      </span>
      <div className="min-w-0">
        <div className="flex flex-wrap items-baseline gap-x-2">
          <a
            href={s.url}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-1 text-[15px] font-semibold text-zinc-900 hover:text-brand"
          >
            {s.name}
            <ArrowUpRight aria-hidden className="size-3.5 text-zinc-400" />
          </a>
          <span className="font-mono text-[12px] text-zinc-400">{s.category}</span>
        </div>
        <p className="mt-0.5 text-[14px] leading-snug text-zinc-700">{s.tagline}</p>
        {s.description ? (
          <p className="mt-1 max-w-2xl text-[13px] leading-relaxed text-zinc-500">{s.description}</p>
        ) : null}
        <p className="mt-1.5 truncate font-mono text-[12px] text-zinc-400">
          {s.submitterName} · {s.submitterEmail} · {ago(s.createdAt)}
          {tags.length ? ` · ${tags.join(", ")}` : ""}
        </p>
      </div>
      {s.status === "new" ? (
        <div className="col-start-2 flex items-center gap-2 sm:col-start-3 sm:self-start">
          <button
            type="button"
            disabled={busy}
            onClick={() => onAct(s.id, "approveSubmission")}
            className="inline-flex h-8 items-center gap-1.5 bg-brand px-3 text-[13px] font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-50"
          >
            <Check aria-hidden className="size-3.5" />
            Approve
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => onAct(s.id, "rejectSubmission")}
            className="inline-flex h-8 items-center gap-1.5 border border-zinc-300 px-3 text-[13px] font-medium text-zinc-600 transition-colors hover:border-zinc-500 hover:text-zinc-900 disabled:opacity-50"
          >
            <X aria-hidden className="size-3.5" />
            Reject
          </button>
        </div>
      ) : null}
    </li>
  );
}

function ago(iso: string): string {
  const mins = Math.max(0, Math.round((Date.now() - Date.parse(iso)) / 60_000));
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.round(mins / 60);
  if (hours < 48) return `${hours}h ago`;
  return `${Math.round(hours / 24)}d ago`;
}

function Stat({ label, value, accent }: { label: string; value: number; accent?: boolean }) {
  return (
    <div className="px-4 py-4 first:pl-0">
      <dt className="text-[12px] font-medium uppercase tracking-wide text-zinc-500">{label}</dt>
      <dd className={"mt-1 font-mono text-[1.75rem] leading-none tabular-nums " + (accent ? "text-brand" : "text-zinc-900")}>
        {value}
      </dd>
    </div>
  );
}

function OwnerOnly({ email }: { email: string }) {
  return (
    <div className="border border-dashed border-zinc-300 px-6 py-12 text-center">
      <h1 className="text-lg font-semibold">This dashboard is owner-only</h1>
      <p className="mx-auto mt-2 max-w-md text-sm text-zinc-500">
        You are signed in as <span className="font-medium text-zinc-700">{email || "this account"}</span>.
        Only the directory owner can review submissions. Set{" "}
        <code className="bg-zinc-100 px-1.5 py-0.5 font-mono text-[12px]">
          PYLON_OWNER_EMAIL={email || "you@yourdirectory.com"}
        </code>{" "}
        in <code className="bg-zinc-100 px-1.5 py-0.5 font-mono text-[12px]">.env</code>, restart, and
        reload. Or sign in with the owner account.
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
      <summary
        aria-label="Account"
        className="flex size-7 cursor-pointer select-none list-none items-center justify-center bg-zinc-900 font-mono text-[12px] font-semibold text-white marker:hidden [&::-webkit-details-marker]:hidden"
      >
        {initial}
      </summary>
      <div className="absolute right-0 top-full z-40 mt-2 w-60 border border-zinc-200 bg-white py-1 shadow-[0_16px_40px_-20px_rgba(0,0,0,0.3)]">
        <div className="truncate border-b border-zinc-100 px-3 py-2 font-mono text-[12px] text-zinc-700">
          {email || "Signed in"}
        </div>
        <button
          type="button"
          onClick={onSignOut}
          className="flex w-full items-center gap-2 px-3 py-2 text-left text-[13px] text-zinc-700 transition-colors hover:bg-zinc-50"
        >
          <LogOut aria-hidden className="size-3.5" />
          Sign out
        </button>
      </div>
    </details>
  );
}

function Skeleton() {
  return (
    <div className="space-y-8">
      <div className="h-8 w-48 animate-pulse bg-zinc-100" />
      <div className="h-20 animate-pulse bg-zinc-50" />
      <div className="space-y-3">
        {[0, 1, 2, 3].map((i) => (
          <div key={i} className="h-16 animate-pulse bg-zinc-50" />
        ))}
      </div>
    </div>
  );
}
