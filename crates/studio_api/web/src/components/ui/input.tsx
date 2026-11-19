import * as React from "react";
import { Input as InputPart } from "@ui-sdk/react/input";

import { cn } from "@/lib/utils";

export type InputProps = Omit<
	React.ComponentProps<typeof InputPart>,
	"className"
> & { className?: string };

/** A one-line text field. It accepts every native input attribute. */
function Input({ className, ...props }: InputProps) {
	return (
		<InputPart
			className={cn(
				// 16px text on a narrow window: iOS zooms the page for a smaller field.
				"h-7 w-full min-w-0 rounded-md border border-input bg-background px-2 text-[13px] text-foreground transition-colors duration-100 max-md:h-9 max-md:text-base",
				"placeholder:text-subtle-foreground",
				"focus-visible:border-ring focus-visible:outline-1 focus-visible:outline-offset-0",
				"disabled:cursor-not-allowed disabled:opacity-50 read-only:bg-muted",
				className,
			)}
			{...props}
		/>
	);
}

export { Input };
