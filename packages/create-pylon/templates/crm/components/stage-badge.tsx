import React from "react";
import { cn } from "@/lib/utils";
import { stageById } from "@/lib/pipeline";

/** The colour dot for each stage, shared by the badge and the board columns. */
export const STAGE_DOT: Record<string, string> = {
  lead: "bg-stage-lead",
  qualified: "bg-stage-qualified",
  proposal: "bg-stage-proposal",
  negotiation: "bg-stage-negotiation",
  won: "bg-stage-won",
  lost: "bg-stage-lost",
};

/**
 * A stage as a dot plus a label. A dot carries the status at a glance in a
 * dense row without the visual weight of a filled chip on every line.
 */
export function StageBadge({
  stage,
  className,
}: {
  stage: string;
  className?: string;
}) {
  const meta = stageById(stage);
  return (
    <span className={cn("inline-flex items-center gap-1.5 whitespace-nowrap", className)}>
      <span
        className={cn("size-1.5 shrink-0 rounded-full", STAGE_DOT[stage] ?? "bg-muted-foreground")}
      />
      <span className="text-muted-foreground">{meta?.label ?? stage}</span>
    </span>
  );
}
