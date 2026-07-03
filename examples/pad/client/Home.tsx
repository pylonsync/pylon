"use client";

import React, { useEffect, useRef, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { FileText, Plus, Users } from "lucide-react";

interface DocRow {
  id: string;
  title: string;
  createdAt: string;
  updatedAt: string;
}

export function Home({ userId }: { userId: string }) {
  const { data: docs, loading } = db.useQuery("Doc", {
    orderBy: { createdAt: "desc" },
    limit: 100,
  });
  const [creating, setCreating] = useState(false);
  const seeded = useRef(false);

  // First-boot seed: create the welcome doc so the list is never empty.
  useEffect(() => {
    if (loading || seeded.current) return;
    if ((docs ?? []).length === 0) {
      seeded.current = true;
      void callFn("seedPad", {}).catch(() => {
        seeded.current = false; // retry on next render if it failed
      });
    }
  }, [loading, docs]);

  async function createDoc() {
    if (creating) return;
    setCreating(true);
    try {
      const now = new Date().toISOString();
      const id = await db.insert("Doc", {
        title: "Untitled",
        content: "# Untitled\n\nStart typing…",
        createdBy: userId,
        createdAt: now,
        updatedAt: now,
      });
      window.location.assign(`/d/${id}`);
    } catch {
      setCreating(false);
    }
  }

  const rows = (docs ?? []) as unknown as DocRow[];

  return (
    <div className="mx-auto max-w-2xl px-6 py-16">
      <div className="mb-2 flex items-center gap-2 text-zinc-400">
        <Users size={16} />
        <span className="text-xs tracking-wide uppercase">
          Collaborative markdown on Pylon
        </span>
      </div>
      <h1 className="text-3xl font-bold tracking-tight">Pad</h1>
      <p className="mt-2 text-[15px] leading-relaxed text-zinc-600">
        Open a document in two windows and type — keystrokes merge live
        through a text CRDT. One entity, two pages, one binary.
      </p>

      <button
        onClick={createDoc}
        disabled={creating}
        className="mt-6 inline-flex h-10 items-center gap-2 rounded-lg bg-zinc-900 px-4 text-sm font-medium text-white transition-colors hover:bg-zinc-700 disabled:opacity-60"
      >
        <Plus size={16} />
        {creating ? "Creating…" : "New document"}
      </button>

      <div className="mt-10 space-y-2">
        {loading && rows.length === 0 ? (
          <p className="text-sm text-zinc-400">Loading…</p>
        ) : null}
        {rows.map((d) => (
          <a
            key={d.id}
            href={`/d/${d.id}`}
            className="flex items-center gap-3 rounded-lg border border-zinc-200 bg-white px-4 py-3 transition-colors hover:border-zinc-300 hover:bg-zinc-50"
          >
            <FileText size={16} className="shrink-0 text-zinc-400" />
            <span className="truncate text-sm font-medium">
              {d.title || "Untitled"}
            </span>
            <span className="ml-auto shrink-0 text-xs text-zinc-400">
              {new Date(d.updatedAt).toLocaleDateString()}
            </span>
          </a>
        ))}
      </div>

      <p className="mt-12 text-xs text-zinc-400">
        Live demo of{" "}
        <a
          href="https://www.pylonsync.com"
          className="underline underline-offset-2"
        >
          Pylon
        </a>{" "}
        — source at{" "}
        <a
          href="https://github.com/pylonsync/pylon/tree/main/examples/pad"
          className="underline underline-offset-2"
        >
          examples/pad
        </a>
        .
      </p>
    </div>
  );
}
