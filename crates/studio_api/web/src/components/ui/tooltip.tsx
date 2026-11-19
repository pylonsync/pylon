import * as React from "react";
import {
	Tooltip as TooltipRoot,
	TooltipContent,
	TooltipTrigger,
} from "@ui-sdk/react/tooltip";

import { cn } from "@/lib/utils";

export interface TooltipProps {
	/** The hint text. */
	label: React.ReactNode;
	/** The control the hint belongs to. It must be one focusable element. */
	children: React.ReactElement;
	side?: React.ComponentProps<typeof TooltipContent>["side"];
	className?: string;
}

/**
 * Shows a short hint when the pointer rests on a control or focus reaches it.
 * The hint names a control that has no text, such as an icon button.
 */
function Tooltip({ label, children, side = "top", className }: TooltipProps) {
	return (
		<TooltipRoot delay={400}>
			<TooltipTrigger render={children} />
			<TooltipContent
				side={side}
				sideOffset={6}
				className={cn(
					"z-50 max-w-xs rounded bg-foreground px-1.5 py-1 text-[11px] leading-tight text-background shadow-md",
					"transition-opacity duration-100 data-starting-style:opacity-0 data-ending-style:opacity-0 motion-reduce:transition-none",
					className,
				)}
			>
				{label}
			</TooltipContent>
		</TooltipRoot>
	);
}

export { Tooltip };
