import * as React from "react";
import {
  Tooltip,
  TooltipContent as TooltipContentPart,
  TooltipTrigger,
} from "@ui-sdk/react/tooltip";
import { cn, type StyledProps } from "../utils";

export { Tooltip, TooltipTrigger };

export function TooltipContent({
  className,
  sideOffset = 4,
  ...props
}: StyledProps<typeof TooltipContentPart>) {
  return (
    <TooltipContentPart
      sideOffset={sideOffset}
      className={cn(
        "z-50 rounded-md border bg-popover px-3 py-1.5 text-xs text-popover-foreground shadow-md",
        "origin-(--transform-origin) transition-[opacity,scale] duration-150 data-starting-style:scale-95 data-starting-style:opacity-0 data-ending-style:scale-95 data-ending-style:opacity-0 motion-reduce:transition-none",
        className,
      )}
      {...props}
    />
  );
}
