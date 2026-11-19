import * as React from "react";
import {
	Tabs as TabsPart,
	TabsList as TabsListPart,
	TabsPanel as TabsPanelPart,
	TabsTab as TabsTabPart,
} from "@ui-sdk/react/tabs";

import { cn } from "@/lib/utils";

type StringClassName<T> = Omit<T, "className"> & { className?: string };

function Tabs({
	className,
	...props
}: StringClassName<React.ComponentProps<typeof TabsPart>>) {
	return (
		<TabsPart className={cn("flex min-h-0 flex-col", className)} {...props} />
	);
}

/** The row of tabs. Give it an `aria-label`. */
function TabsList({
	className,
	...props
}: StringClassName<React.ComponentProps<typeof TabsListPart>>) {
	return (
		<TabsListPart
			className={cn("flex h-9 shrink-0 items-stretch gap-3 border-b px-3", className)}
			{...props}
		/>
	);
}

function TabsTab({
	className,
	...props
}: StringClassName<React.ComponentProps<typeof TabsTabPart>>) {
	return (
		<TabsTabPart
			className={cn(
				"-mb-px inline-flex cursor-default items-center gap-1.5 border-b-2 border-transparent px-0.5 text-[13px] whitespace-nowrap text-muted-foreground transition-colors duration-100 hover:text-foreground data-selected:border-foreground data-selected:font-medium data-selected:text-foreground data-disabled:opacity-50",
				className,
			)}
			{...props}
		/>
	);
}

function TabsPanel({
	className,
	...props
}: StringClassName<React.ComponentProps<typeof TabsPanelPart>>) {
	return (
		<TabsPanelPart
			className={cn("min-h-0 flex-1 outline-none", className)}
			{...props}
		/>
	);
}

export { Tabs, TabsList, TabsPanel, TabsTab };
