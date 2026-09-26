import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { BRAND } from "@/lib/brand";
import { DealView } from "./deal-view";

export const metadata: Metadata = {
  title: `Deal · ${BRAND.name}`,
  robots: "noindex",
};

/** `app/deals/[id]/page.tsx` → `/deals/:id`. */
export default function DealPage({ params }: PageProps<{ id: string }>) {
  return <DealView dealId={params.id} />;
}
