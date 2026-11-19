import * as React from "react";
import {
  Dialog,
  DialogBackdrop,
  DialogClose,
  DialogContent as DialogContentPart,
  DialogDescription as DialogDescriptionPart,
  DialogTitle as DialogTitlePart,
  DialogTrigger,
} from "@ui-sdk/react/dialog";
import { X } from "lucide-react";
import { cn, type StyledProps } from "../utils";

export { Dialog, DialogClose, DialogTrigger };

/** The dimmed layer behind a dialog. `DialogContent` renders one. */
export function DialogOverlay({ className, ...props }: StyledProps<typeof DialogBackdrop>) {
  return (
    <DialogBackdrop
      className={cn(
        "fixed inset-0 z-50 bg-black/50 transition-opacity duration-200 data-starting-style:opacity-0 data-ending-style:opacity-0 motion-reduce:transition-none",
        className,
      )}
      {...props}
    />
  );
}

/**
 * The dialog panel. It renders its own overlay and a close button, so a
 * caller writes only the title, the description and the body.
 */
export function DialogContent({
  className,
  children,
  ...props
}: Omit<StyledProps<typeof DialogContentPart>, "children"> & { children?: React.ReactNode }) {
  return (
    <>
      <DialogOverlay />
      <DialogContentPart
        className={cn(
          "fixed inset-0 z-50 m-auto grid h-fit max-h-[calc(100dvh-2rem)] w-[calc(100%-2rem)] max-w-lg gap-4 overflow-y-auto overscroll-contain rounded-lg border bg-background p-6 text-foreground shadow-lg outline-none",
          "transition-[opacity,scale] duration-200 data-starting-style:scale-95 data-starting-style:opacity-0 data-ending-style:scale-95 data-ending-style:opacity-0 motion-reduce:transition-none",
          className,
        )}
        {...props}
      >
        {children}
        <DialogClose
          aria-label="Close"
          className="absolute right-4 top-4 rounded-sm opacity-70 transition-opacity hover:opacity-100 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
        >
          <X className="size-4" />
        </DialogClose>
      </DialogContentPart>
    </>
  );
}

export function DialogHeader({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div className={cn("flex flex-col space-y-1.5 text-center sm:text-left", className)} {...props} />
  );
}

export function DialogFooter({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn("flex flex-col-reverse sm:flex-row sm:justify-end sm:space-x-2", className)}
      {...props}
    />
  );
}

export function DialogTitle({ className, ...props }: StyledProps<typeof DialogTitlePart>) {
  return (
    <DialogTitlePart
      className={cn("text-lg font-semibold leading-none tracking-tight", className)}
      {...props}
    />
  );
}

export function DialogDescription({ className, ...props }: StyledProps<typeof DialogDescriptionPart>) {
  return <DialogDescriptionPart className={cn("text-sm text-muted-foreground", className)} {...props} />;
}
