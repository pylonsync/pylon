import * as React from "react";
import {
  ScrollArea as ScrollAreaPart,
  ScrollAreaContent,
  ScrollAreaCorner,
  ScrollAreaScrollbar,
  ScrollAreaThumb,
  ScrollAreaViewport,
} from "@ui-sdk/react/scroll-area";
import { cn, type StyledProps } from "../utils";

/** A scroll container with a styled vertical scrollbar. Give it a height. */
export function ScrollArea({
  className,
  children,
  ...props
}: Omit<StyledProps<typeof ScrollAreaPart>, "children"> & { children?: React.ReactNode }) {
  return (
    <ScrollAreaPart className={cn("relative overflow-hidden", className)} {...props}>
      <ScrollAreaViewport className="h-full w-full rounded-[inherit]">
        <ScrollAreaContent>{children}</ScrollAreaContent>
      </ScrollAreaViewport>
      <ScrollBar />
      <ScrollAreaCorner />
    </ScrollAreaPart>
  );
}

/** A scrollbar for one axis of the `ScrollArea` around it. */
export function ScrollBar({
  className,
  orientation = "vertical",
  ...props
}: Omit<StyledProps<typeof ScrollAreaScrollbar>, "children">) {
  return (
    <ScrollAreaScrollbar
      orientation={orientation}
      className={cn(
        "flex touch-none select-none p-px opacity-0 transition-opacity data-visible:opacity-100",
        orientation === "vertical" && "w-2.5 flex-col border-l border-l-transparent",
        orientation === "horizontal" && "h-2.5 border-t border-t-transparent",
        className,
      )}
      {...props}
    >
      <ScrollAreaThumb className="relative rounded-full bg-border" />
    </ScrollAreaScrollbar>
  );
}
