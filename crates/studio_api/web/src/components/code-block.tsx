import * as React from "react";

import { CopyButton } from "@/components/copy-button";
import { cn } from "@/lib/utils";

/** Preformatted text, such as JSON, with a copy button. */
export function CodeBlock({
	value,
	className,
	...props
}: Omit<React.ComponentProps<"pre">, "children"> & { value: string }) {
	return (
		<div className="relative min-w-0">
			<pre
				className={cn(
					"overflow-auto rounded-md border bg-muted px-2.5 py-2 text-xs leading-relaxed",
					className,
				)}
				{...props}
			>
				{value}
			</pre>
			<CopyButton
				value={value}
				label="Copy"
				className="absolute top-1.5 right-1.5 bg-muted"
			/>
		</div>
	);
}
