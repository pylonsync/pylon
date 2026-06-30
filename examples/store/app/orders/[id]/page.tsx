import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { RequireAuth } from "../../../client/RequireAuth";
import { OrderDetail } from "../../../client/OrderDetail";

export const metadata: Metadata = {
  title: "Order · Pylon Store",
  description: "Order status + shipping timeline.",
};

// `/orders/<id>` — order detail with the live shipping timeline. The order
// data is sync-backed + owner-scoped, so this mounts the client island
// behind the real-account gate.
export default function Page({ params }: PageProps<{ id: string }>) {
  return (
    <RequireAuth>
      <OrderDetail id={params.id} />
    </RequireAuth>
  );
}
