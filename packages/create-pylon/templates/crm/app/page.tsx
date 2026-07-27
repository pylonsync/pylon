import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { PipelineView } from "./pipeline-view";

export const metadata: Metadata = {
  title: "Pipeline",
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
