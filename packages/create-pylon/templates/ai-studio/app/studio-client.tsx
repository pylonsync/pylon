"use client";

import React, { useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { EnsureGuest } from "@pylonsync/client";
import { siteConfig } from "@/lib/site.config";
import type { GenerationKind, GenerationRow } from "@/lib/studio";

// The studio — a client island. `Generation` is an owner-scoped entity read with
// `db.useQuery`, so your gallery is private and updates live: the generate action
// inserts a "pending" row (it appears instantly), runs the provider call on the
// server, then flips the row to the finished result — and that change syncs to
// every open tab. <EnsureGuest> gives an anonymous visitor a session so they can
// generate + own their gallery; the API key stays on the server.

export function Studio() {
  return (
    <EnsureGuest fallback={<GallerySkeleton />}>
      <StudioInner />
    </EnsureGuest>
  );
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

  async function generate() {
    const p = prompt.trim();
    if (!p || busy) return;
    setBusy(true);
    setError(null);
    setPrompt("");
    // Fire the action: the pending card shows up live via useQuery while it runs;
    // we only await to surface input errors + re-enable the button.
    try {
      await callFn("generate", { kind, prompt: p });
    } catch (e) {
      const msg = e instanceof Error ? e.message : "Couldn't start the generation.";
      setError(/INVALID_ARGS/.test(msg) ? "Enter a prompt (up to 1000 characters)." : msg);
    } finally {
      setBusy(false);
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
                {!k.wired ? <span className="ml-1 text-[10px] text-zinc-400">stub</span> : null}
              </button>
            ))}
          </div>
          <button
            type="button"
            onClick={generate}
            disabled={busy || !prompt.trim()}
            className="inline-flex h-9 items-center gap-1.5 rounded-full bg-brand px-5 text-[13.5px] font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-40"
          >
            {busy ? "Generating…" : "Generate"}
          </button>
        </div>
      </div>

      {error ? <p className="mt-3 text-[13px] text-red-600">{error}</p> : null}

      {/* Examples (only before anything's been made) */}
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

      {/* Gallery */}
      <div className="mt-8 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {generations.map((g) => (
          <GenerationCard key={g.id} g={g} />
        ))}
      </div>
    </div>
  );
}

function GenerationCard({ g }: { g: GenerationRow }) {
  return (
    <div className="overflow-hidden rounded-2xl border border-zinc-200 bg-white">
      <div className="relative grid aspect-square place-items-center bg-paper">
        <Media g={g} />
        {g.demo && g.status === "done" ? (
          <span className="absolute left-2 top-2 rounded-full border border-dashed border-zinc-300 bg-white/80 px-2 py-0.5 text-[10px] font-medium text-zinc-500">
            demo
          </span>
        ) : null}
        <span className="absolute right-2 top-2 rounded-full bg-white/85 px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide text-zinc-500 backdrop-blur">
          {g.kind}
        </span>
      </div>
      <div className="p-3">
        <p className="line-clamp-2 text-[13px] leading-snug text-zinc-600">{g.prompt}</p>
      </div>
    </div>
  );
}

function Media({ g }: { g: GenerationRow }) {
  if (g.status === "pending") {
    return (
      <div className="flex flex-col items-center gap-2 text-zinc-400">
        <Spinner />
        <span className="text-[12px]">Generating…</span>
      </div>
    );
  }
  if (g.status === "failed") {
    return (
      <div className="px-5 text-center text-[12px] text-red-500">
        {g.error || "Generation failed."}
      </div>
    );
  }
  // done
  if (g.kind === "image" && g.resultUrl) {
    // eslint-disable-next-line @next/next/no-img-element
    return <img src={g.resultUrl} alt={g.prompt} className="size-full object-cover" />;
  }
  if (g.kind === "audio") {
    return g.resultUrl ? (
      <div className="w-full px-4">
        <AudioWave />
        <audio controls src={g.resultUrl} className="mt-3 w-full" />
      </div>
    ) : (
      <DemoNote text="Audio needs OPENAI_API_KEY — set it to hear real speech." />
    );
  }
  if (g.kind === "video") {
    return <DemoNote text="Video isn't wired yet — add a provider in functions/generate.ts (Replicate / fal / Runway)." />;
  }
  return <DemoNote text="No result." />;
}

function DemoNote({ text }: { text: string }) {
  return (
    <div className="flex flex-col items-center gap-2 px-5 text-center text-zinc-400">
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

function GallerySkeleton() {
  return (
    <div className="mx-auto max-w-5xl px-4 py-8">
      <div className="h-28 w-full animate-pulse rounded-2xl bg-zinc-100" />
      <div className="mt-8 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {Array.from({ length: 3 }).map((_, i) => (
          <div key={i} className="aspect-square animate-pulse rounded-2xl bg-zinc-100" />
        ))}
      </div>
    </div>
  );
}
