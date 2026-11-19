import * as React from "react";
import { Switch as SwitchPart, SwitchThumb } from "@ui-sdk/react/switch";
import { cn, type StyledProps } from "../utils";

export function Switch({ className, ...props }: Omit<StyledProps<typeof SwitchPart>, "children">) {
  return (
    <SwitchPart
      className={cn(
        "peer inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full border-2 border-transparent transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background data-disabled:cursor-not-allowed data-disabled:opacity-50 data-checked:bg-primary data-unchecked:bg-input",
        className,
      )}
      {...props}
    >
      <SwitchThumb className="pointer-events-none block size-4 rounded-full bg-background shadow-lg ring-0 transition-transform data-checked:translate-x-4 data-unchecked:translate-x-0 rtl:data-checked:-translate-x-4" />
    </SwitchPart>
  );
}
