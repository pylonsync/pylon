import * as React from "react";
import { Label as LabelPart } from "@ui-sdk/react/label";

import { cn } from "@/lib/utils";

export type LabelProps = Omit<
	React.ComponentProps<typeof LabelPart>,
	"className"
> & { className?: string };

/** The name of a form control. Connect it with `htmlFor`. */
function Label({ className, ...props }: LabelProps) {
	return (
		<LabelPart
			className={cn("text-xs font-medium text-foreground", className)}
			{...props}
		/>
	);
}

export { Label };
