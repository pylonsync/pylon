import * as React from "react";
import { Input as InputPart } from "@ui-sdk/react/input";

import { cn } from "@/lib/utils";

function Input({
  className,
  ...props
}: Omit<React.ComponentProps<typeof InputPart>, "className"> & {
  className?: string;
}) {
  return (
    <InputPart
      className={cn(
        "flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors",
        "placeholder:text-muted-foreground",
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
        "disabled:cursor-not-allowed disabled:opacity-50",
        // Date/time inputs render a native picker indicator that ignores the
        // text color; nudge it so it stays visible in dark mode.
        "[&::-webkit-calendar-picker-indicator]:opacity-60 dark:[&::-webkit-calendar-picker-indicator]:invert",
        "file:border-0 file:bg-transparent file:text-sm file:font-medium",
        className,
      )}
      {...props}
    />
  );
}

export { Input };
