import * as React from "react";

import { cn } from "@/lib/utils";

/** A placeholder block for content that is loading. */
function Skeleton({ className, ...props }: React.ComponentProps<"div">) {
	return (
		<div
			aria-hidden="true"
			className={cn(
				"h-3 animate-pulse rounded bg-accent motion-reduce:animate-none",
				className,
			)}
			{...props}
		/>
	);
}

export { Skeleton };
