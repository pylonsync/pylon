"use client";

import React, { useEffect, useState } from "react";
import { db } from "@pylonsync/react";
import { bootClient, type Listing } from "./market";

export function LiveListingStatus({
  listingId,
  initialStatus,
  mode = "text",
}: {
  listingId: string;
  initialStatus: Listing["status"];
  mode?: "text" | "overlay";
}) {
  const [mounted, setMounted] = useState(false);
  useEffect(() => {
    bootClient();
    setMounted(true);
  }, []);

  if (!mounted) {
    return initialStatus === "sold" ? (
      mode === "overlay" ? <SoldOverlay /> : <>Sold</>
    ) : mode === "text" ? (
      <>Open to offers</>
    ) : null;
  }

  return (
    <LiveStatus
      listingId={listingId}
      initialStatus={initialStatus}
      mode={mode}
    />
  );
}

function LiveStatus({
  listingId,
  initialStatus,
  mode,
}: {
  listingId: string;
  initialStatus: Listing["status"];
  mode: "text" | "overlay";
}) {
  const { data } = db.useQueryOne<Listing>("Listing", listingId);
  const sold = (data?.status ?? initialStatus) === "sold";
  if (mode === "overlay") return sold ? <SoldOverlay /> : null;
  return <>{sold ? "Sold" : "Open to offers"}</>;
}

function SoldOverlay() {
  return (
    <span className="absolute inset-0 grid place-items-center bg-black/55 text-2xl font-semibold text-white">
      Sold
    </span>
  );
}
