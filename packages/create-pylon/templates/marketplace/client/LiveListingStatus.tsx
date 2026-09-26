"use client";

import React, { useEffect, useState } from "react";
import { db } from "@pylonsync/react";
import { bootClient, type Listing } from "./market";

type Mode = "text" | "pill" | "overlay";

// A listing's sale status, live. Renders the server value first, then follows
// the Listing row over sync, so a sale in another tab flips it in place.
export function LiveListingStatus({
  listingId,
  initialStatus,
  mode = "text",
}: {
  listingId: string;
  initialStatus: Listing["status"];
  mode?: Mode;
}) {
  const [mounted, setMounted] = useState(false);
  useEffect(() => {
    bootClient();
    setMounted(true);
  }, []);

  if (!mounted) return <Status sold={initialStatus === "sold"} mode={mode} />;
  return (
    <LiveStatus listingId={listingId} initialStatus={initialStatus} mode={mode} />
  );
}

function LiveStatus({
  listingId,
  initialStatus,
  mode,
}: {
  listingId: string;
  initialStatus: Listing["status"];
  mode: Mode;
}) {
  const { data } = db.useQueryOne<Listing>("Listing", listingId);
  return <Status sold={(data?.status ?? initialStatus) === "sold"} mode={mode} />;
}

function Status({ sold, mode }: { sold: boolean; mode: Mode }) {
  if (mode === "overlay") {
    return sold ? (
      <span className="market-rise absolute inset-0 grid place-items-center bg-black/45">
        <span className="rounded-full bg-white px-5 py-2 font-display text-2xl text-neutral-900">
          Sold
        </span>
      </span>
    ) : null;
  }
  if (mode === "pill") {
    return sold ? (
      <span className="inline-flex items-center rounded-full bg-foreground px-3 py-1 text-xs font-semibold text-background">
        Sold
      </span>
    ) : (
      <span className="inline-flex items-center gap-1.5 rounded-full bg-success/12 px-3 py-1 text-xs font-semibold text-success-foreground">
        <span className="size-1.5 rounded-full bg-success" aria-hidden="true" />
        Available
      </span>
    );
  }
  return <>{sold ? "Sold" : "Available"}</>;
}
