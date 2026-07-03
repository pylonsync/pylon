"use client";

import React, { useEffect, useState } from "react";
import { db, useRoom } from "@pylonsync/react";
import { useCollabTextarea } from "@pylonsync/loro";
import { ArrowLeft, Check, Link2 } from "lucide-react";
import { identityFor } from "./bootstrap";
import { Markdown } from "./markdown";

export function Editor({ docId, userId }: { docId: string; userId: string }) {
  const { data: doc, loading } = db.useQueryOne("Doc", docId);
  // The collaborative body: minimal-splice diffs out, caret-preserving
  // merges in. `value` doubles as the live source for the preview pane.
  const { ref, value, onInput } = useCollabTextarea("Doc", docId, "content");

  const me = identityFor(userId);
  // Presence: everyone with this doc open, as colored avatars.
  const { peers } = useRoom(`pad:${docId}`, userId, {
    initialPresence: { name: me.name, color: me.color },
  });

  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const t = setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(t);
  }, [copied]);

  async function copyLink() {
    try {
      await navigator.clipboard.writeText(window.location.href);
      setCopied(true);
    } catch {
      // Clipboard unavailable (permissions) — the URL bar still works.
    }
  }

  async function renameDoc(title: string) {
    try {
      await db.update("Doc", docId, {
        title,
        updatedAt: new Date().toISOString(),
      });
    } catch {
      // Best-effort; the CRDT body is the document of record.
    }
  }

  if (!loading && !doc) {
    return (
      <div className="mx-auto max-w-2xl px-6 py-16">
        <p className="text-sm text-zinc-500">Document not found.</p>
        <a href="/" className="mt-2 inline-block text-sm underline">
          Back to documents
        </a>
      </div>
    );
  }

  const title = (doc as { title?: string } | null)?.title ?? "";

  return (
    <div className="flex h-screen flex-col">
      <header className="flex h-14 shrink-0 items-center gap-3 border-b border-zinc-200 bg-white px-4">
        <a
          href="/"
          className="rounded p-1.5 text-zinc-400 transition-colors hover:bg-zinc-100 hover:text-zinc-700"
          aria-label="Back to documents"
        >
          <ArrowLeft size={16} />
        </a>
        <input
          defaultValue={title}
          key={title || docId}
          placeholder="Untitled"
          onBlur={(e) => {
            const next = e.target.value.trim();
            if (next && next !== title) void renameDoc(next);
          }}
          className="w-64 rounded border border-transparent bg-transparent px-2 py-1 text-sm font-semibold outline-none transition-colors focus:border-zinc-300 focus:bg-white"
        />

        <div className="ml-auto flex items-center gap-3">
          {/* Presence avatars: me + everyone else in the room. */}
          <div className="flex items-center -space-x-1.5">
            <Avatar name={me.name} color={me.color} me />
            {peers.map((p: { user_id: string; data: Record<string, unknown> }) => {
              const data = p.data as { name?: string; color?: string };
              const fallback = identityFor(p.user_id);
              return (
                <Avatar
                  key={p.user_id}
                  name={data.name ?? fallback.name}
                  color={data.color ?? fallback.color}
                />
              );
            })}
          </div>
          <span className="text-xs text-zinc-400">
            {peers.length === 0
              ? "just you — open this URL in a second window"
              : `${peers.length + 1} editing`}
          </span>
          <button
            onClick={copyLink}
            className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-zinc-300 bg-white px-3 text-xs font-medium transition-colors hover:bg-zinc-50"
          >
            {copied ? <Check size={13} /> : <Link2 size={13} />}
            {copied ? "Copied" : "Copy link"}
          </button>
        </div>
      </header>

      <main className="grid min-h-0 flex-1 grid-cols-1 md:grid-cols-2">
        <textarea
          ref={ref}
          defaultValue={value}
          onInput={onInput}
          spellCheck={false}
          placeholder="Write markdown…"
          className="h-full w-full resize-none border-r border-zinc-200 bg-white p-6 font-mono text-[13.5px] leading-relaxed outline-none"
        />
        <div className="hidden h-full overflow-y-auto bg-zinc-50 p-6 md:block">
          <Markdown source={value} />
        </div>
      </main>
    </div>
  );
}

function Avatar({
  name,
  color,
  me = false,
}: {
  name: string;
  color: string;
  me?: boolean;
}) {
  return (
    <span
      title={me ? `${name} (you)` : name}
      style={{ backgroundColor: color }}
      className="inline-flex h-7 w-7 items-center justify-center rounded-full border-2 border-white text-[11px] font-semibold text-white"
    >
      {name.slice(0, 1)}
    </span>
  );
}
