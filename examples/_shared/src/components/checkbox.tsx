import * as React from "react";
import { Checkbox as CheckboxPart, CheckboxIndicator } from "@ui-sdk/react/checkbox";
import { Check } from "lucide-react";
import { cn, type StyledProps } from "../utils";

export function Checkbox({ className, ...props }: Omit<StyledProps<typeof CheckboxPart>, "children">) {
  return (
    <CheckboxPart
      className={cn(
        "peer flex size-4 shrink-0 items-center justify-center rounded-sm border border-primary focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring data-disabled:cursor-not-allowed data-disabled:opacity-50 data-checked:bg-primary data-checked:text-primary-foreground data-indeterminate:bg-primary data-indeterminate:text-primary-foreground",
        className,
      )}
      {...props}
    >
      <CheckboxIndicator aria-hidden="true" className="flex items-center justify-center text-current">
        <Check className="size-3.5" />
      </CheckboxIndicator>
    </CheckboxPart>
  );
}
