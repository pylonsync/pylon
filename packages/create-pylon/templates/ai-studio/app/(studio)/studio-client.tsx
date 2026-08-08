"use client";

import React, { useEffect, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { siteConfig } from "@/lib/site.config";
import type { GenerationKind, GenerationRow } from "@/lib/studio";

// The studio — a client island, rendered only for a SIGNED-IN user (the page
// redirects anyone else to /login). `Generation` is an owner-scoped entity read
// with `db.useQuery`, so your gallery is private to your account and updates
// live: the generate mutation inserts a "pending" row (it appears instantly), a
// background job runs the provider call, then flips the row to the finished
// result — and that change syncs to every open tab. The API token stays on the
// server.

export function Studio() {
  return <StudioInner />;
}

function StudioInner() {
  const { studio } = siteConfig;
  const { data: generations } = db.useQuery<GenerationRow>("Generation", {
    orderBy: { createdAt: "desc" },
  });
  const [prompt, setPrompt] = useState("");
  const [kind, setKind] = useState<GenerationKind>("image");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [openId, setOpenId] = useState<string | null>(null);

  // Keep the open modal pointed at the live row so it reflects status changes.
  const open = generations.find((g) => g.id === openId) ?? null;

  async function generate(p = prompt, k = kind) {
    const text = p.trim();
    if (!text || busy) return;
    setBusy(true);
    setError(null);
    setPrompt("");
    try {
      await callFn("generate", { kind: k, prompt: text });
    } catch (e) {
      const msg = e instanceof Error ? e.message : "Couldn't start the generation.";
      setError(/INVALID_ARGS/.test(msg) ? "Enter a prompt (up to 1000 characters)." : msg);
    } finally {
      setBusy(false);
    }
  }

  function reuse(g: GenerationRow) {
    setPrompt(g.prompt);
    setKind((["image", "audio", "video"].includes(g.kind) ? g.kind : "image") as GenerationKind);
    setOpenId(null);
    window.scrollTo({ top: 0, behavior: "smooth" });
  }

  async function remove(id: string) {
    setOpenId(null);
    try {
      await db.delete("Generation", id);
    } catch {
      /* ignore — the row may already be gone */
    }
  }

  return (
    <div className="mx-auto max-w-5xl px-4 py-8">
      {/* Prompt bar */}
      <div className="rounded-2xl border border-zinc-200 bg-white p-4 shadow-sm">
        <textarea
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              generate();
            }
          }}
          rows={2}
          placeholder={studio.inputPlaceholder}
          aria-label="Prompt"
          className="w-full resize-none bg-transparent text-[15px] text-zinc-900 outline-none placeholder:text-zinc-400"
        />
        <div className="mt-2 flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-1 rounded-full bg-zinc-100 p-1">
            {studio.kinds.map((k) => (
              <button
                key={k.id}
                type="button"
                onClick={() => setKind(k.id)}
                className={
                  "rounded-full px-3 py-1.5 text-[13px] font-medium transition-colors " +
                  (kind === k.id ? "bg-white text-zinc-900 shadow-sm" : "text-zinc-500 hover:text-zinc-800")
                }
              >
                {k.label}
              </button>
            ))}
          </div>
          <button
            type="button"
            onClick={() => generate()}
            disabled={busy || !prompt.trim()}
            className="inline-flex h-9 items-center gap-1.5 rounded-full bg-brand px-5 text-[13.5px] font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-40"
          >
            {busy ? "Generating…" : "Generate"}
          </button>
        </div>
      </div>

      {error ? <p className="mt-3 text-[13px] text-red-600">{error}</p> : null}

      {generations.length === 0 ? (
        <div className="mt-5 flex flex-wrap gap-2">
          {studio.examples.map((ex) => (
            <button
              key={ex}
              type="button"
              onClick={() => setPrompt(ex)}
              className="rounded-full border border-zinc-200 bg-white px-3 py-1.5 text-[12.5px] text-zinc-500 transition-colors hover:border-brand hover:text-zinc-800"
            >
              {ex}
            </button>
          ))}
        </div>
      ) : null}

      <div className="mt-8 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {generations.map((g) => (
          <GenerationCard key={g.id} g={g} onOpen={() => setOpenId(g.id)} />
        ))}
      </div>

      {open ? (
        <GenerationModal
          g={open}
          onClose={() => setOpenId(null)}
          onReuse={() => reuse(open)}
          onDelete={() => remove(open.id)}
        />
      ) : null}
    </div>
  );
}

function GenerationCard({ g, onOpen }: { g: GenerationRow; onOpen: () => void }) {
  const done = g.status === "done";
  return (
    <button
      type="button"
      onClick={onOpen}
      className="group block overflow-hidden rounded-2xl border border-zinc-200 bg-white text-left transition-shadow hover:shadow-md focus:outline-none focus:ring-2 focus:ring-brand/30"
    >
      <div className="relative grid aspect-square place-items-center bg-paper">
        <Media g={g} />
        {g.demo && done ? (
          <span className="absolute left-2 top-2 rounded-full border border-dashed border-zinc-300 bg-white/80 px-2 py-0.5 text-[10px] font-medium text-zinc-500">
            demo
          </span>
        ) : null}
        <span className="absolute right-2 top-2 rounded-full bg-white/85 px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide text-zinc-500 backdrop-blur">
          {g.kind}
        </span>
      </div>
      <div className="flex items-center justify-between gap-2 p-3">
        <p className="line-clamp-1 text-[13px] text-zinc-600">{g.prompt}</p>
        <span className="shrink-0 text-[11px] text-zinc-400">{relativeTime(g.createdAt)}</span>
      </div>
    </button>
  );
}

function GenerationModal({
  g,
  onClose,
  onReuse,
  onDelete,
}: {
  g: GenerationRow;
  onClose: () => void;
  onReuse: () => void;
  onDelete: () => void;
}) {
  const [copied, setCopied] = useState(false);
  // Close on Escape.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  async function copyPrompt() {
    try {
      await navigator.clipboard.writeText(g.prompt);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      /* clipboard unavailable */
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-zinc-900/50 p-4"
      onClick={onClose}
    >
      <div
        className="flex max-h-[90vh] w-full max-w-lg flex-col overflow-hidden rounded-2xl bg-white shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="grid max-h-[55vh] place-items-center overflow-hidden bg-paper">
          <Media g={g} full />
        </div>
        <div className="flex-1 overflow-y-auto p-5">
          <div className="flex items-center gap-2">
            <span className="rounded-full bg-zinc-100 px-2 py-0.5 text-[11px] font-medium uppercase tracking-wide text-zinc-500">
              {g.kind}
            </span>
            <StatusPill status={g.status} demo={g.demo} />
            <span className="text-[12px] text-zinc-400">{relativeTime(g.createdAt)}</span>
          </div>
          <p className="mt-3 text-[14px] leading-relaxed text-zinc-800">{g.prompt}</p>
          {g.error ? <p className="mt-2 text-[13px] text-red-600">{g.error}</p> : null}

          <div className="mt-5 flex flex-wrap gap-2">
            {g.status === "done" && g.resultUrl ? (
              <a
                href={g.resultUrl}
                download
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex h-9 items-center gap-1.5 rounded-lg bg-brand px-4 text-[13px] font-medium text-white transition-opacity hover:opacity-90"
              >
                Download
              </a>
            ) : null}
            <button
              type="button"
              onClick={copyPrompt}
              className="inline-flex h-9 items-center rounded-lg border border-zinc-300 px-4 text-[13px] font-medium text-zinc-700 transition-colors hover:bg-zinc-50"
            >
              {copied ? "Copied ✓" : "Copy prompt"}
            </button>
            <button
              type="button"
              onClick={onReuse}
              className="inline-flex h-9 items-center rounded-lg border border-zinc-300 px-4 text-[13px] font-medium text-zinc-700 transition-colors hover:bg-zinc-50"
            >
              Reuse prompt
            </button>
            <button
              type="button"
              onClick={onDelete}
              className="ml-auto inline-flex h-9 items-center rounded-lg border border-zinc-200 px-4 text-[13px] font-medium text-zinc-500 transition-colors hover:border-red-300 hover:text-red-600"
            >
              Delete
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function Media({ g, full }: { g: GenerationRow; full?: boolean }) {
  if (g.status === "pending" || g.status === "processing") {
    return (
      <div className="flex flex-col items-center gap-2 py-10 text-zinc-400">
        <Spinner />
        <span className="text-[12px]">{g.status === "processing" ? "Generating…" : "Queued…"}</span>
      </div>
    );
  }
  if (g.status === "failed") {
    return <div className="px-5 py-10 text-center text-[12px] text-red-500">{g.error || "Generation failed."}</div>;
  }
  if (g.kind === "image" && g.resultUrl) {
    // eslint-disable-next-line @next/next/no-img-element
    return <img src={g.resultUrl} alt={g.prompt} className={full ? "max-h-[55vh] w-auto object-contain" : "size-full object-cover"} />;
  }
  if (g.kind === "audio") {
    return g.resultUrl ? (
      <div className="w-full px-5 py-8">
        <AudioWave />
        <audio controls src={g.resultUrl} className="mt-3 w-full" />
      </div>
    ) : (
      <DemoNote text="Add REPLICATE_API_TOKEN to generate real audio." />
    );
  }
  if (g.kind === "video") {
    return g.resultUrl ? (
      <video controls src={g.resultUrl} className={full ? "max-h-[55vh] w-auto" : "size-full object-cover"} />
    ) : (
      <DemoNote text="Add REPLICATE_API_TOKEN to generate real video." />
    );
  }
  return <DemoNote text="No result." />;
}

function StatusPill({ status, demo }: { status: string; demo: boolean }) {
  if (status === "done") {
    return demo ? (
      <span className="rounded-full border border-dashed border-zinc-300 px-2 py-0.5 text-[10px] font-medium text-zinc-500">demo</span>
    ) : (
      <span className="rounded-full bg-green-50 px-2 py-0.5 text-[10px] font-medium text-green-700">done</span>
    );
  }
  if (status === "failed") {
    return <span className="rounded-full bg-red-50 px-2 py-0.5 text-[10px] font-medium text-red-600">failed</span>;
  }
  return <span className="rounded-full bg-amber-50 px-2 py-0.5 text-[10px] font-medium capitalize text-amber-700">{status}</span>;
}

function DemoNote({ text }: { text: string }) {
  return (
    <div className="flex flex-col items-center gap-2 px-5 py-10 text-center text-zinc-400">
      <svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden>
        <rect x="3" y="3" width="18" height="18" rx="2" />
        <path d="M3 9h18M9 21V9" />
      </svg>
      <span className="text-[12px] leading-snug">{text}</span>
    </div>
  );
}

function AudioWave() {
  return (
    <div className="flex items-end justify-center gap-1">
      {[10, 22, 14, 28, 18, 26, 12].map((h, i) => (
        <span key={i} className="w-1.5 rounded-full bg-brand/60" style={{ height: h }} />
      ))}
    </div>
  );
}

function Spinner() {
  return (
    <svg className="size-6 animate-spin text-brand" viewBox="0 0 24 24" fill="none" aria-hidden>
      <circle cx="12" cy="12" r="9" stroke="currentColor" strokeWidth="3" opacity="0.2" />
      <path d="M21 12a9 9 0 0 0-9-9" stroke="currentColor" strokeWidth="3" strokeLinecap="round" />
    </svg>
  );
}

// "just now" / "5m ago" / "3h ago" / "2d ago" from an ISO timestamp.
function relativeTime(iso: string): string {
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return "";
  const s = Math.max(0, Math.floor((Date.now() - t) / 1000));
  if (s < 45) return "just now";
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}
