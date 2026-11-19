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
				"fixed inset-0 z-50 bg-overlay transition-opacity duration-150 data-starting-style:opacity-0 data-ending-style:opacity-0 motion-reduce:transition-none",
				className,
			)}
			{...props}
		/>
	);
}

/**
 * The dialog panel, centered on the page. It renders the backdrop and a close
 * button. Put a `DialogHeader`, a `DialogBody` and a `DialogFooter` in it.
 */
function DialogContent({
	className,
	children,
	...props
}: Omit<
	StringClassName<React.ComponentProps<typeof DialogContentPart>>,
	"children"
> & { children?: React.ReactNode }) {
	return (
		<>
			<DialogBackdrop />
			<DialogContentPart
				className={cn(
					"fixed inset-0 z-50 m-auto flex h-fit max-h-[calc(100dvh-2rem)] w-[calc(100%-2rem)] max-w-md flex-col overflow-hidden rounded-lg border border-border bg-popover text-popover-foreground shadow-[0_24px_48px_-12px_rgb(0_0_0/0.3)] outline-none",
					"transition-[opacity,scale] duration-150 data-starting-style:scale-[0.98] data-starting-style:opacity-0 data-ending-style:scale-[0.98] data-ending-style:opacity-0 motion-reduce:transition-none",
					className,
				)}
				{...props}
			>
				{children}
				<DialogClose
					aria-label="Close"
					className="absolute top-2.5 right-2.5 flex size-6 items-center justify-center rounded text-muted-foreground hover:bg-accent hover:text-foreground"
				>
					<X className="size-3.5" />
				</DialogClose>
			</DialogContentPart>
		</>
	);
}

function DialogHeader({ className, ...props }: React.ComponentProps<"div">) {
	return (
		<div
			className={cn("flex flex-col gap-1 px-4 pt-3.5 pr-10 pb-3", className)}
			{...props}
		/>
	);
}

function DialogBody({ className, ...props }: React.ComponentProps<"div">) {
	return (
		<div
			className={cn("min-h-0 flex-1 overflow-y-auto px-4 pb-4", className)}
			{...props}
		/>
	);
}

function DialogFooter({ className, ...props }: React.ComponentProps<"div">) {
	return (
		<div
			className={cn(
				"flex shrink-0 items-center justify-end gap-2 border-t bg-muted/50 px-4 py-2.5",
				className,
			)}
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
			className={cn("text-sm font-semibold", className)}
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
			className={cn("text-[13px] text-muted-foreground", className)}
			{...props}
		/>
	);
}

export {
	Dialog,
	DialogBackdrop,
	DialogBody,
	DialogClose,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	DialogTrigger,
};
