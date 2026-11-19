import * as React from "react";
import {
  Dialog as Sheet,
  DialogBackdrop,
  DialogClose as SheetClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DialogTrigger as SheetTrigger,
} from "@ui-sdk/react/dialog";
import { X } from "lucide-react";
import { cn, type StyledProps } from "../utils";

export { Sheet, SheetClose, SheetTrigger };

const sideClasses = {
  right:
    "inset-y-0 right-0 left-auto h-dvh w-full max-w-md border-l data-starting-style:translate-x-full data-ending-style:translate-x-full",
  left: "inset-y-0 left-0 right-auto h-dvh w-full max-w-md border-r data-starting-style:-translate-x-full data-ending-style:-translate-x-full",
  top: "inset-x-0 top-0 bottom-auto w-full max-w-none border-b data-starting-style:-translate-y-full data-ending-style:-translate-y-full",
  bottom:
    "inset-x-0 bottom-0 top-auto w-full max-w-none border-t data-starting-style:translate-y-full data-ending-style:translate-y-full",
} as const;

/**
 * A panel that slides in from one edge of the screen. It is a modal dialog:
 * focus stays in the panel, and Escape or a press on the overlay closes it.
 */
export function SheetContent({
  side = "right",
  className,
  children,
  ...props
}: Omit<StyledProps<typeof DialogContent>, "children"> & {
  side?: keyof typeof sideClasses;
  children?: React.ReactNode;
}) {
  return (
    <>
      <DialogBackdrop className="fixed inset-0 z-50 bg-black/50 transition-opacity duration-300 data-starting-style:opacity-0 data-ending-style:opacity-0 motion-reduce:transition-none" />
      <DialogContent
        className={cn(
          "fixed z-50 m-0 flex max-h-none flex-col gap-4 bg-background text-foreground shadow-lg outline-none transition-transform duration-300 ease-in-out motion-reduce:transition-none",
          sideClasses[side],
          className,
        )}
        {...props}
      >
        <SheetClose
          aria-label="Close"
          className="absolute right-4 top-4 rounded-sm opacity-70 transition-opacity hover:opacity-100 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <X className="size-4" />
        </SheetClose>
        {children}
      </DialogContent>
    </>
  );
}

export function SheetHeader({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("flex flex-col space-y-2 px-6 pt-6 text-left", className)} {...props} />;
}

export function SheetFooter({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("mt-auto flex flex-col gap-2 border-t px-6 py-4", className)} {...props} />;
}

export function SheetTitle({ className, ...props }: StyledProps<typeof DialogTitle>) {
  return <DialogTitle className={cn("text-lg font-semibold text-foreground", className)} {...props} />;
}

export function SheetDescription({ className, ...props }: StyledProps<typeof DialogDescription>) {
  return <DialogDescription className={cn("text-sm text-muted-foreground", className)} {...props} />;
}
