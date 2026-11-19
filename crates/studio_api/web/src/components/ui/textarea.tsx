import * as React from "react";
import { Textarea as TextareaPart } from "@ui-sdk/react/textarea";

import { cn } from "@/lib/utils";

export type TextareaProps = Omit<
	React.ComponentProps<typeof TextareaPart>,
	"className"
> & { className?: string };

/** A text field with more than one line. */
function Textarea({ className, ...props }: TextareaProps) {
	return (
		<TextareaPart
			className={cn(
				"w-full min-w-0 rounded-md border border-input bg-background px-2 py-1.5 text-[13px] text-foreground transition-colors duration-100 max-md:text-base",
				"placeholder:text-subtle-foreground",
				"focus-visible:border-ring focus-visible:outline-1 focus-visible:outline-offset-0",
				"disabled:cursor-not-allowed disabled:opacity-50 read-only:bg-muted",
				className,
			)}
			{...props}
		/>
	);
}

export { Textarea };
