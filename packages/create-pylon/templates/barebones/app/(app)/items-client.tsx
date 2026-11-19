"use client";

import React, { useState } from "react";
import { db } from "@pylonsync/react";
import { EnsureGuest } from "@pylonsync/client";
import { Button } from "@/components/ui/button";

export interface Item {
  id: string;
  text: string;
}

// `<EnsureGuest>` mints a guest session on first load (POST /api/auth/guest)
// so the list works with no login — every visitor becomes their own user and
// their items stay private (the policy in app.ts gates reads/writes to the
// owner).
export function ItemList() {
  return (
    <EnsureGuest fallback={<Skeleton />}>
      <List />
    </EnsureGuest>
  );
}

// `db.useQuery` is a LIVE subscription — it re-renders the instant an Item is
// added, here or in another tab. `db.insert` is OPTIMISTIC: it shows the row
// immediately and syncs in the background, rolling back if the policy rejects.
function List() {
  const [text, setText] = useState("");
  const { data: items, loading } = db.useQuery<Item>("Item");

  async function addItem(e: React.FormEvent) {
    e.preventDefault();
    const value = text.trim();
    if (!value) return;
    setText("");
    // No userId sent — `field.owner()` stamps it server-side.
    await db.insert("Item", { text: value });
  }

  return (
    <div className="space-y-5">
      <form onSubmit={addItem} className="flex items-center gap-2">
        <input
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder="Add an item…"
          aria-label="Item"
          autoComplete="off"
          className="flex h-10 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm outline-none focus-visible:ring-1 focus-visible:ring-ring"
        />
        <Button type="submit">Add</Button>
      </form>

      {loading && items.length === 0 ? (
        <Skeleton />
      ) : items.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          Nothing yet — add an item above. It appears instantly and syncs; open
          a second tab to watch it arrive live.
        </p>
      ) : (
        <ul className="space-y-2">
          {items.map((item) => (
            <li
              key={item.id}
              className="flex items-center justify-between gap-3 rounded-md border px-3 py-2.5 text-sm"
            >
              <span className="truncate">{item.text}</span>
              <button
                type="button"
                aria-label="Delete item"
                onClick={() => db.delete("Item", item.id)}
                className="text-muted-foreground/40 transition-colors hover:text-red-600"
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

function Skeleton() {
  return (
    <ul className="space-y-2" aria-hidden>
      {[0, 1, 2].map((i) => (
        <li key={i} className="rounded-md border px-3 py-2.5">
          <span className="block h-4 w-2/3 rounded bg-muted" />
        </li>
      ))}
    </ul>
  );
}
