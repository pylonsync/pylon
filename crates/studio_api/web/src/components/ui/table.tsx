import * as React from "react";

import { cn } from "@/lib/utils";

/**
 * Dense table parts: 32px rows, 13px text, tabular numbers.
 *
 * `Table` does not scroll. Put it in an element with `overflow-auto` and the
 * header row stays at the top of that element.
 */
function Table({ className, ...props }: React.ComponentProps<"table">) {
	return (
		<table
			className={cn(
				"w-full border-separate border-spacing-0 text-left text-[13px] tabular-nums",
				className,
			)}
			{...props}
		/>
	);
}

function TableHeader({ className, ...props }: React.ComponentProps<"thead">) {
	return <thead className={cn("sticky top-0 z-20", className)} {...props} />;
}

function TableBody({ className, ...props }: React.ComponentProps<"tbody">) {
	return <tbody className={className} {...props} />;
}

function TableRow({ className, ...props }: React.ComponentProps<"tr">) {
	return (
		<tr
			className={cn(
				"group/row outline-none [&>td]:bg-background hover:[&>td]:bg-muted data-[selected=true]:[&>td]:bg-selected data-[current=true]:[&>td]:bg-selected focus-visible:[&>td]:bg-muted",
				className,
			)}
			{...props}
		/>
	);
}

function TableHead({ className, ...props }: React.ComponentProps<"th">) {
	return (
		<th
			scope="col"
			className={cn(
				"h-8 border-b bg-muted px-3 text-xs font-medium whitespace-nowrap text-muted-foreground",
				className,
			)}
			{...props}
		/>
	);
}

function TableCell({ className, ...props }: React.ComponentProps<"td">) {
	return (
		<td
			className={cn(
				"h-8 max-w-[28rem] truncate border-b px-3 whitespace-nowrap",
				className,
			)}
			{...props}
		/>
	);
}

export { Table, TableBody, TableCell, TableHead, TableHeader, TableRow };
