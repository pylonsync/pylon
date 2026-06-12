"use client";

import React, { useState } from "react";
import { db } from "@pylonsync/react";
import { useAuth } from "@pylonsync/client";
import { Button } from "@/components/ui/button";

export interface Note {
  id: string;
  body: string;
  done: boolean;
}

// The interactive dashboard. `db.useQuery` is a LIVE subscription — it
// re-renders the instant a Note is added or toggled, in this tab or another.
// `db.insert` / `db.update` / `db.delete` are OPTIMISTIC: they apply to the
// local store immediately (zero-latency UI) and sync in the background,
// rolling back automatically if a policy rejects the write.
//
// `initial` are the rows the server rendered into the HTML (see page.tsx).
// We show them on the first paint — before the local store has hydrated — so
// there's no empty flash, then hand off to the live data. Server-rendered for
// the first byte, local-first realtime after.
export function Dashboard({ initial }: { initial: Note[] }) {
  const { signOut } = useAuth();
  const [body, setBody] = useState("");
  const { data: live, loading } = db.useQuery<Note>("Note");
  const notes = !loading || live.length > 0 ? live : initial;

  async function addNote(e: React.FormEvent) {
    e.preventDefault();
    const text = body.trim();
    if (!text) return;
    setBody("");
    // We don't send ownerId — `field.owner()` stamps it from the session
    // server-side and rejects any forged value, so this optimistic insert is
    // safe.
    await db.insert("Note", { body: text, done: false });
  }

  async function onSignOut() {
    // Clears the server session (DELETE /api/auth/session → the cookie is
    // cleared), then we land back on the public homepage.
    await signOut();
    window.location.assign("/");
  }

  return (
    <div className="space-y-5">
      <div className="flex items-center justify-end">
        <Button variant="ghost" size="sm" onClick={onSignOut}>
          Sign out
        </Button>
      </div>

      <form onSubmit={addNote} className="flex items-center gap-2">
        <input
          value={body}
          onChange={(e) => setBody(e.target.value)}
          placeholder="Write a note…"
          aria-label="Note"
          className="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm outline-none focus-visible:ring-1 focus-visible:ring-ring"
        />
        <Button type="submit">Add</Button>
      </form>

      {notes.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          No notes yet — add one above. It appears instantly (optimistic) and
          syncs; open this page in a second tab to watch it arrive live.
        </p>
      ) : (
        <ul className="space-y-2">
          {notes.map((note) => (
            <li
              key={note.id}
              className="flex items-center gap-3 rounded-md border px-3 py-2 text-sm"
            >
              <button
                type="button"
                aria-label={note.done ? "Mark not done" : "Mark done"}
                onClick={() =>
                  db.update("Note", note.id, { done: !note.done })
                }
                className={
                  note.done
                    ? "text-emerald-600"
                    : "text-muted-foreground/50 hover:text-muted-foreground"
                }
              >
                {note.done ? "✓" : "○"}
              </button>
              <span
                className={
                  note.done
                    ? "flex-1 line-through text-muted-foreground"
                    : "flex-1"
                }
              >
                {note.body}
              </span>
              <button
                type="button"
                aria-label="Delete note"
                onClick={() => db.delete("Note", note.id)}
                className="text-muted-foreground/40 hover:text-red-600"
              >
                ✕
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
