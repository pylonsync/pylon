import * as React from "react";

import { cn } from "@/lib/utils";

/**
 * One row of numbers. Put `Stat` items in it. The items share the width and
 * wrap to two columns on a narrow window.
 */
export function StatStrip({ className, ...props }: React.ComponentProps<"dl">) {
	return (
		<dl
			className={cn(
				"grid shrink-0 grid-cols-2 border-b sm:auto-cols-fr sm:grid-flow-col sm:grid-cols-none",
				"[&>div]:border-b sm:[&>div]:border-b-0 [&>div:not(:last-child)]:sm:border-r [&>div:nth-child(odd)]:max-sm:border-r",
				"-mb-px sm:mb-0",
				className,
			)}
			{...props}
		/>
	);
}

export interface StatProps {
	label: React.ReactNode;
	value: React.ReactNode;
	/** A short note under the value. */
	note?: React.ReactNode;
	tone?: "default" | "good" | "warn" | "bad";
	/** `sm` is for a strip of counts inside a section. */
	size?: "default" | "sm";
	className?: string;
}

const TONE = {
	default: "",
	good: "text-status-green-fg",
	warn: "text-status-amber-fg",
	bad: "text-status-red-fg",
} as const;

export function Stat({
	label,
	value,
	note,
	tone = "default",
	size = "default",
	className,
}: StatProps) {
	return (
		<div className={cn("min-w-0 px-3", size === "sm" ? "py-2" : "py-2.5", className)}>
			<dt className="truncate text-xs text-muted-foreground">{label}</dt>
			<dd
				className={cn(
					"truncate font-semibold tabular-nums",
					size === "sm" ? "text-sm" : "mt-0.5 text-lg leading-6",
					TONE[tone],
				)}
			>
				{value}
			</dd>
			{note != null && (
				<dd className="truncate text-[11px] text-muted-foreground tabular-nums">
					{note}
				</dd>
			)}
		</div>
	);
}
