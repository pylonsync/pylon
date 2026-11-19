import * as React from "react";
import {
	Dialog as Sheet,
	DialogBackdrop,
	DialogClose as SheetClose,
	DialogContent,
	DialogDescription as SheetDescriptionPart,
	DialogTitle as SheetTitlePart,
	DialogTrigger as SheetTrigger,
} from "@ui-sdk/react/dialog";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

type StringClassName<T> = Omit<T, "className"> & { className?: string };

const sheetVariants = cva(
	"fixed inset-y-0 z-50 m-0 flex h-dvh max-h-none max-w-none flex-col bg-background text-foreground outline-none transition-[translate,opacity] duration-150 data-starting-style:opacity-0 data-ending-style:opacity-0 motion-reduce:transition-none",
	{
		variants: {
			side: {
				left: "right-auto left-0 border-r data-starting-style:-translate-x-4 data-ending-style:-translate-x-4",
				right:
					"right-0 left-auto border-l data-starting-style:translate-x-4 data-ending-style:translate-x-4",
			},
		},
		defaultVariants: { side: "right" },
	},
);

export type SheetContentProps = Omit<
	StringClassName<React.ComponentProps<typeof DialogContent>>,
	"children"
> &
	VariantProps<typeof sheetVariants> & {
		/** Dims the page behind the panel. Use it with a modal sheet. */
		backdrop?: boolean;
		children?: React.ReactNode;
	};

/**
 * A panel on the left or right edge of the screen.
 *
 * `<Sheet>` is a dialog: modal by default, so focus stays in the panel. Pass
 * `modal={false}` to `<Sheet>` to keep the page behind it usable. Give the
 * panel a `SheetTitle`.
 */
function SheetContent({
	className,
	side,
	backdrop = true,
	children,
	...props
}: SheetContentProps) {
	return (
		<>
			{backdrop && (
				<DialogBackdrop className="fixed inset-0 z-40 bg-overlay transition-opacity duration-150 data-starting-style:opacity-0 data-ending-style:opacity-0 motion-reduce:transition-none" />
			)}
			<DialogContent
				className={cn(
					sheetVariants({ side }),
					"shadow-[0_0_40px_-8px_rgb(0_0_0/0.25)]",
					className,
				)}
				{...props}
			>
				{children}
			</DialogContent>
		</>
	);
}

function SheetHeader({ className, ...props }: React.ComponentProps<"div">) {
	return (
		<div
			className={cn(
				"flex min-h-11 shrink-0 items-center gap-2 border-b px-3 py-1.5",
				className,
			)}
			{...props}
		/>
	);
}

function SheetBody({ className, ...props }: React.ComponentProps<"div">) {
	return (
		<div
			className={cn("min-h-0 flex-1 overflow-y-auto", className)}
			{...props}
		/>
	);
}

function SheetFooter({ className, ...props }: React.ComponentProps<"div">) {
	return (
		<div
			className={cn(
				"flex shrink-0 items-center justify-end gap-2 border-t px-3 py-2",
				className,
			)}
			{...props}
		/>
	);
}

function SheetTitle({
	className,
	...props
}: StringClassName<React.ComponentProps<typeof SheetTitlePart>>) {
	return (
		<SheetTitlePart
			className={cn("truncate text-[13px] font-semibold", className)}
			{...props}
		/>
	);
}

function SheetDescription({
	className,
	...props
}: StringClassName<React.ComponentProps<typeof SheetDescriptionPart>>) {
	return (
		<SheetDescriptionPart
			className={cn("text-xs text-muted-foreground", className)}
			{...props}
		/>
	);
}

export {
	Sheet,
	SheetBody,
	SheetClose,
	SheetContent,
	SheetDescription,
	SheetFooter,
	SheetHeader,
	SheetTitle,
	SheetTrigger,
};
