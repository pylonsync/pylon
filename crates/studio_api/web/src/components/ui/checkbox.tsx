import * as React from "react";
import {
	Checkbox as CheckboxPart,
	CheckboxIndicator,
} from "@ui-sdk/react/checkbox";
import { Check, Minus } from "lucide-react";

import { cn } from "@/lib/utils";

export type CheckboxProps = Omit<
	React.ComponentProps<typeof CheckboxPart>,
	"className" | "children"
> & { className?: string };

/**
 * A checkbox. Pass `checked="indeterminate"` to show a partial selection.
 * A checkbox with no visible text needs an `aria-label`.
 */
function Checkbox({ className, ...props }: CheckboxProps) {
	return (
		<CheckboxPart
			className={cn(
				"group/checkbox inline-flex size-3.5 shrink-0 cursor-default items-center justify-center rounded-[3px] border border-border-strong bg-background text-primary-foreground transition-colors duration-100",
				"data-checked:border-primary data-checked:bg-primary data-indeterminate:border-primary data-indeterminate:bg-primary",
				"data-disabled:opacity-40",
				className,
			)}
			{...props}
		>
			<CheckboxIndicator aria-hidden="true" className="flex">
				<Check
					className="size-3 group-data-indeterminate/checkbox:hidden"
					strokeWidth={3}
				/>
				<Minus
					className="hidden size-3 group-data-indeterminate/checkbox:block"
					strokeWidth={3}
				/>
			</CheckboxIndicator>
		</CheckboxPart>
	);
}

export { Checkbox };
