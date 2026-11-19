import * as React from "react";

import { cn } from "@/lib/utils";

/** A list of label and value rows. Put `Fact` items in it. */
export function Facts({ className, ...props }: React.ComponentProps<"dl">) {
	return <dl className={cn("pb-2", className)} {...props} />;
}

export function Fact({
	label,
	children,
	className,
}: {
	label: React.ReactNode;
	children: React.ReactNode;
	className?: string;
}) {
	return (
		<div className={cn("flex min-h-8 items-center gap-3 px-3 py-0.5", className)}>
			<dt className="w-32 shrink-0 text-muted-foreground">{label}</dt>
			<dd className="min-w-0 truncate tabular-nums">{children}</dd>
		</div>
	);
}

/** A titled block of JSON or text inside a detail panel. */
export function FactBlock({
	label,
	children,
}: {
	label: React.ReactNode;
	children: React.ReactNode;
}) {
	return (
		<div className="space-y-1 px-3 pb-3">
			<h3 className="text-xs font-medium text-muted-foreground">{label}</h3>
			{children}
		</div>
	);
}
