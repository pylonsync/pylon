import * as React from "react";
import { Button as ButtonPart } from "@ui-sdk/react/button";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

const buttonVariants = cva(
	"inline-flex shrink-0 cursor-default items-center justify-center gap-1.5 rounded-md font-medium whitespace-nowrap select-none transition-colors duration-100 data-disabled:pointer-events-none data-disabled:opacity-50 data-pending:cursor-progress data-pending:opacity-70 [&_svg]:pointer-events-none [&_svg]:size-3.5 [&_svg]:shrink-0",
	{
		variants: {
			variant: {
				default: "bg-primary text-primary-foreground hover:bg-primary/90",
				outline:
					"border border-border-strong bg-background text-foreground hover:bg-accent",
				secondary: "bg-secondary text-secondary-foreground hover:bg-accent",
				ghost: "text-muted-foreground hover:bg-accent hover:text-foreground",
				destructive:
					"bg-destructive text-destructive-foreground hover:bg-destructive/90",
				link: "text-link underline-offset-2 hover:underline",
			},
			size: {
				default: "h-7 px-2.5 text-[13px]",
				sm: "h-6 px-2 text-xs",
				lg: "h-8 px-3 text-[13px]",
				icon: "size-7",
				"icon-sm": "size-6",
			},
		},
		defaultVariants: { variant: "default", size: "default" },
	},
);

export type ButtonProps = Omit<
	React.ComponentProps<typeof ButtonPart>,
	"className"
> &
	VariantProps<typeof buttonVariants> & { className?: string };

/**
 * A button. When `onClick` returns a Promise, the button stays pending
 * (`data-pending`) until the Promise settles, so the action cannot run twice.
 *
 * To render a link with the look of a button, pass the link to `render`:
 * `<Button render={<a href="/studio/logout" />}>Sign out</Button>`.
 */
function Button({ className, variant, size, ...props }: ButtonProps) {
	return (
		<ButtonPart
			className={cn(buttonVariants({ variant, size }), className)}
			{...props}
		/>
	);
}

export { Button };
