"use client";

import * as React from "react";
import {
  Dialog,
  DialogBackdrop as DialogBackdropPart,
  DialogClose,
  DialogContent as DialogContentPart,
  DialogDescription as DialogDescriptionPart,
  DialogTitle as DialogTitlePart,
  DialogTrigger,
} from "@ui-sdk/react/dialog";
import { X } from "lucide-react";

import { cn } from "@/lib/utils";

type StringClassName<T> = Omit<T, "className"> & { className?: string };

function DialogBackdrop({
  className,
  ...props
}: StringClassName<React.ComponentProps<typeof DialogBackdropPart>>) {
  return (
    <DialogBackdropPart
      className={cn(
        "fixed inset-0 z-50 bg-black/50 transition-opacity duration-200 data-starting-style:opacity-0 data-ending-style:opacity-0 motion-reduce:transition-none",
        className,
      )}
      {...props}
    />
  );
}

/**
 * The dialog panel. It renders its own backdrop and a close button, so a
 * caller writes only the title, the description and the body.
 */
function DialogContent({
  className,
  children,
  ...props
}: Omit<StringClassName<React.ComponentProps<typeof DialogContentPart>>, "children"> & {
  children?: React.ReactNode;
}) {
  return (
    <>
      <DialogBackdrop />
      <DialogContentPart
        className={cn(
          "fixed inset-0 z-50 m-auto grid h-fit max-h-[calc(100dvh-2rem)] w-[calc(100%-2rem)] max-w-lg gap-4 overflow-y-auto overscroll-contain rounded-xl border bg-background p-6 text-foreground shadow-lg outline-none",
          "transition-[opacity,scale] duration-200 data-starting-style:scale-95 data-starting-style:opacity-0 data-ending-style:scale-95 data-ending-style:opacity-0 motion-reduce:transition-none",
          className,
        )}
        {...props}
      >
        {children}
        <DialogClose
          aria-label="Close"
          className="absolute right-3 top-3 rounded-md p-1 text-muted-foreground opacity-70 transition-opacity hover:opacity-100 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <X className="size-4" />
        </DialogClose>
      </DialogContentPart>
    </>
  );
}

function DialogHeader({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div className={cn("flex flex-col gap-1.5 text-left", className)} {...props} />
  );
}

function DialogFooter({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn("flex flex-col-reverse gap-2 sm:flex-row sm:justify-end", className)}
      {...props}
    />
  );
}

function DialogTitle({
  className,
  ...props
}: StringClassName<React.ComponentProps<typeof DialogTitlePart>>) {
  return (
    <DialogTitlePart
      className={cn("text-base font-semibold leading-none tracking-tight", className)}
      {...props}
    />
  );
}

function DialogDescription({
  className,
  ...props
}: StringClassName<React.ComponentProps<typeof DialogDescriptionPart>>) {
  return (
    <DialogDescriptionPart
      className={cn("text-sm text-muted-foreground", className)}
      {...props}
    />
  );
}

export {
  Dialog,
  DialogBackdrop,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
};
