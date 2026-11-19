"use client";

import * as React from "react";
import {
  Dialog as Sheet,
  DialogBackdrop,
  DialogClose as SheetClose,
  DialogContent,
  DialogDescription as SheetDescription,
  DialogTitle as SheetTitle,
  DialogTrigger as SheetTrigger,
} from "@ui-sdk/react/dialog";

import { cn } from "@/lib/utils";

interface SheetContentProps
  extends Omit<React.ComponentProps<typeof DialogContent>, "className" | "children"> {
  className?: string;
  /** Classes for the backdrop behind the panel. */
  backdropClassName?: string;
  children?: React.ReactNode;
}

/**
 * A panel that slides in from the left edge of the screen, for navigation on
 * small screens. It is a modal dialog: focus stays in the panel, and Escape or
 * a press on the backdrop closes it. Give it a `SheetTitle`.
 */
function SheetContent({
  className,
  backdropClassName,
  children,
  ...props
}: SheetContentProps) {
  return (
    <>
      <DialogBackdrop
        className={cn(
          "fixed inset-0 z-40 bg-black/40 transition-opacity duration-200 data-starting-style:opacity-0 data-ending-style:opacity-0 motion-reduce:transition-none",
          backdropClassName,
        )}
      />
      <DialogContent
        className={cn(
          "fixed inset-y-0 left-0 right-auto z-50 m-0 flex h-dvh max-h-none w-[min(300px,86vw)] max-w-none bg-background text-foreground shadow-xl outline-none",
          "transition-transform duration-200 data-starting-style:-translate-x-full data-ending-style:-translate-x-full motion-reduce:transition-none",
          className,
        )}
        {...props}
      >
        {children}
      </DialogContent>
    </>
  );
}

export {
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetTitle,
  SheetTrigger,
};
