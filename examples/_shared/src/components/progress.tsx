import * as React from "react";
import {
  Progress as ProgressPart,
  ProgressIndicator,
  ProgressTrack,
} from "@ui-sdk/react/progress";
import { cn, type StyledProps } from "../utils";

/**
 * A progress bar. `value` runs from `min` (0) to `max` (100). A null `value`
 * means the progress is unknown, and the bar pulses.
 */
export function Progress({ className, ...props }: Omit<StyledProps<typeof ProgressPart>, "children">) {
  return (
    <ProgressPart className={cn("block w-full", className)} {...props}>
      <ProgressTrack className="relative block h-2 w-full overflow-hidden rounded-full bg-secondary">
        <ProgressIndicator className="absolute inset-y-0 bg-primary transition-[width] data-indeterminate:inset-x-0 data-indeterminate:animate-pulse" />
      </ProgressTrack>
    </ProgressPart>
  );
}
