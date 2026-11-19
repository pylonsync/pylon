import * as React from "react";
import { PanelLeft } from "lucide-react";

import { Button } from "@/components/ui/button";
import { getExtensionLayoutSlot } from "@/lib/extensions";
import { useShell } from "@/layout/shell";
import { cn } from "@/lib/utils";

export interface PageProps {
	/** The page title. It renders as the `h1`. */
	title: React.ReactNode;
	/** A link back to the parent page, for a page that is nested in another. */
	parent?: React.ReactNode;
	/** Short facts next to the title, such as a row count. */
	meta?: React.ReactNode;
	/** Buttons on the right of the top bar. */
	actions?: React.ReactNode;
	children?: React.ReactNode;
	className?: string;
}

/**
 * One Studio page: a top bar with the title and the page actions, then the
 * content. The content fills the height that is left. Put a `PageScroll`
 * in it, or a layout that scrolls by itself.
 */
export function Page({
	title,
	parent,
	meta,
	actions,
	children,
	className,
}: PageProps) {
	const { navToggle, navHidden, toggleNav } = useShell();
	const headerActions = getExtensionLayoutSlot("headerActions");
	return (
		<div className={cn("flex min-h-0 min-w-0 flex-1 flex-col", className)}>
			<header className="flex h-11 shrink-0 items-center gap-2 border-b px-3">
				{navToggle && (
					<Button
						variant="ghost"
						size="icon"
						className="-ml-1"
						aria-label={navHidden ? "Show the sidebar" : "Hide the sidebar"}
						onClick={toggleNav}
					>
						<PanelLeft />
					</Button>
				)}
				<div className="flex min-w-0 flex-1 items-center gap-1.5 overflow-hidden">
					{parent && (
						<>
							<span className="shrink-0 text-[13px] text-muted-foreground">
								{parent}
							</span>
							<span aria-hidden="true" className="text-subtle-foreground">
								/
							</span>
						</>
					)}
					<h1 className="shrink-0 text-[13px] font-semibold">{title}</h1>
					{meta != null && meta !== false && (
						<span className="min-w-0 truncate text-xs text-muted-foreground tabular-nums">
							{meta}
						</span>
					)}
				</div>
				<div className="flex shrink-0 items-center gap-1.5">
					{actions}
					{headerActions && React.createElement(headerActions)}
				</div>
			</header>
			{children}
		</div>
	);
}

/** The scrolling content area of a page. */
export function PageScroll({
	className,
	...props
}: React.ComponentProps<"div">) {
	return (
		<div className={cn("min-h-0 flex-1 overflow-auto", className)} {...props} />
	);
}

/** A row of controls directly under the top bar or above a table. */
export function Toolbar({ className, ...props }: React.ComponentProps<"div">) {
	return (
		<div
			className={cn(
				"flex min-h-10 shrink-0 flex-wrap items-center gap-1.5 border-b px-3 py-1.5",
				className,
			)}
			{...props}
		/>
	);
}

export interface SectionProps
	extends Omit<React.ComponentProps<"section">, "title"> {
	title: React.ReactNode;
	/** Short facts or controls on the right of the section title. */
	aside?: React.ReactNode;
}

/** A titled block of a page. Sections stack with a rule between them. */
export function Section({
	title,
	aside,
	className,
	children,
	...props
}: SectionProps) {
	return (
		<section className={cn("border-b", className)} {...props}>
			<div className="flex h-9 items-center gap-2 px-3">
				<h2 className="text-xs font-semibold">{title}</h2>
				{aside != null && (
					<div className="ml-auto flex items-center gap-2 text-xs text-muted-foreground">
						{aside}
					</div>
				)}
			</div>
			{children}
		</section>
	);
}
