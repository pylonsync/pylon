import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { BRAND } from "@/lib/brand";
import { PipelineView } from "./pipeline-view";

export const metadata: Metadata = {
  title: `Pipeline · ${BRAND.name}`,
  robots: "noindex",
};

/**
 * `/` — the pipeline board.
 *
 */
export default function PipelinePage({ searchParams }: PageProps) {
  return (
    <PipelineView
      // The ⌘K "New deal" action navigates here with ?new=deal.
      openNew={searchParams?.new === "deal"}
    />
  );
}
