import * as React from "react";
import { Switch as SwitchPart, SwitchThumb } from "@ui-sdk/react/switch";

import { cn } from "@/lib/utils";

export type SwitchProps = Omit<
	React.ComponentProps<typeof SwitchPart>,
	"className" | "children"
> & { className?: string };

/** An on/off control. A switch with no visible text needs an `aria-label`. */
function Switch({ className, ...props }: SwitchProps) {
	return (
		<SwitchPart
			className={cn(
				"inline-flex h-4 w-7 shrink-0 cursor-default items-center rounded-full bg-border-strong p-0.5 transition-colors duration-100 data-checked:bg-primary data-disabled:opacity-50",
				className,
			)}
			{...props}
		>
			<SwitchThumb className="size-3 rounded-full bg-background shadow-sm transition-transform duration-100 data-checked:translate-x-3 motion-reduce:transition-none" />
		</SwitchPart>
	);
}

export { Switch };
