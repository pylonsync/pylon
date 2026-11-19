import * as React from "react";
import { Toggle, ToggleGroup as ToggleGroupPart } from "@ui-sdk/react/toggle";
import { cn, type StyledProps } from "../utils";

/**
 * A row of toggles that share one value. The value is an array of the pressed
 * items' values. One item is pressed at a time unless `multiple` is set.
 */
export function ToggleGroup({ className, ...props }: StyledProps<typeof ToggleGroupPart>) {
  return (
    <ToggleGroupPart className={cn("flex items-center justify-center gap-1", className)} {...props} />
  );
}

export function ToggleGroupItem({ className, ...props }: StyledProps<typeof Toggle>) {
  return (
    <Toggle
      className={cn(
        "inline-flex h-8 items-center justify-center gap-2 rounded-md px-2 text-sm font-medium transition-colors hover:bg-muted hover:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring data-disabled:pointer-events-none data-disabled:opacity-50 data-pressed:bg-accent data-pressed:text-accent-foreground",
        className,
      )}
      {...props}
    />
  );
}
