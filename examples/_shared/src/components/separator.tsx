import * as React from "react";
import { Separator as SeparatorPart } from "@ui-sdk/react/separator";
import { cn, type StyledProps } from "../utils";

/** A line between groups of content. It is hidden from screen readers unless `decorative` is false. */
export function Separator({ className, decorative = true, ...props }: StyledProps<typeof SeparatorPart>) {
  return (
    <SeparatorPart
      decorative={decorative}
      className={cn(
        "shrink-0 bg-border data-[orientation=horizontal]:h-px data-[orientation=horizontal]:w-full data-[orientation=vertical]:h-full data-[orientation=vertical]:w-px",
        className,
      )}
      {...props}
    />
  );
}
