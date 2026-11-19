import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

const badgeVariants = cva(
	"inline-flex h-[18px] w-fit shrink-0 items-center gap-1 rounded px-1.5 text-[11px] font-medium whitespace-nowrap",
	{
		variants: {
			variant: {
				gray: "bg-status-gray-bg text-status-gray-fg",
				green: "bg-status-green-bg text-status-green-fg",
				red: "bg-status-red-bg text-status-red-fg",
				amber: "bg-status-amber-bg text-status-amber-fg",
				blue: "bg-status-blue-bg text-status-blue-fg",
				outline: "border border-border text-muted-foreground",
			},
		},
		defaultVariants: { variant: "gray" },
	},
);

export type BadgeProps = React.ComponentProps<"span"> &
	VariantProps<typeof badgeVariants> & {
		/** Shows a dot in the color of the variant before the text. */
		dot?: boolean;
	};

/** A short status label. */
function Badge({ className, variant, dot, children, ...props }: BadgeProps) {
	return (
		<span className={cn(badgeVariants({ variant }), className)} {...props}>
			{dot && <span className="size-1.5 rounded-full bg-current" />}
			{children}
		</span>
	);
}

export { Badge };
