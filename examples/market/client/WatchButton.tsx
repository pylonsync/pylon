"use client";

import React, { useEffect, useState } from "react";
import { db } from "@pylonsync/react";
import { Heart } from "lucide-react";
import { cn } from "@pylonsync/example-ui/utils";
import { bootClient, readIdentity, type Watch } from "./market";

// Heart toggle that saves a listing to your private watchlist. Self-contained
// (no provider needed): boots the client, reads identity, and toggles the
// Watch row with optimistic db.insert / db.delete — the live query below flips
// the fill instantly. Hidden for signed-out visitors (watchlists are a
// logged-in feature). The mounted gate keeps db.useQuery off the SSR pass.
export function WatchButton(props: {
  listingId: string;
  listingTitle: string;
  className?: string;
}) {
  const [mounted, setMounted] = useState(false);
  useEffect(() => {
    bootClient();
    setMounted(true);
  }, []);
  if (!mounted) return null;
  return <Inner {...props} />;
}

function Inner({
  listingId,
  listingTitle,
  className,
}: {
  listingId: string;
  listingTitle: string;
  className?: string;
}) {
  const [identity, setIdentity] = useState(() => readIdentity());
  useEffect(() => {
    const on = () => setIdentity(readIdentity());
    window.addEventListener("pylon-auth-changed", on);
    window.addEventListener("storage", on);
    return () => {
      window.removeEventListener("pylon-auth-changed", on);
      window.removeEventListener("storage", on);
    };
  }, []);

  // Policy scopes reads to the caller, so this returns only MY watch (if any)
  // for this listing.
  const { data } = db.useQuery<Watch>("Watch", { where: { listingId } });
  const mine = identity ? data?.find((w) => w.userId === identity.userId) : undefined;
  const watched = !!mine;

  // No heart for signed-out visitors.
  if (!identity) return null;

  function toggle(e: React.MouseEvent) {
    // Stop the click from bubbling into the surrounding card link.
    e.preventDefault();
    e.stopPropagation();
    if (mine) {
      void db.delete("Watch", mine.id);
    } else {
      void db.insert("Watch", { userId: identity!.userId, listingId, listingTitle });
    }
  }

  return (
    <button
      type="button"
      onClick={toggle}
      aria-pressed={watched}
      aria-label={watched ? "Remove from watchlist" : "Save to watchlist"}
      title={watched ? "Saved" : "Save to watchlist"}
      className={cn(
        "grid size-9 place-items-center rounded-full bg-background/80 backdrop-blur transition hover:bg-background",
        className,
      )}
    >
      <Heart
        className={cn(
          "size-5 transition",
          watched ? "fill-rose-500 text-rose-500" : "text-foreground/70",
        )}
      />
    </button>
  );
}
