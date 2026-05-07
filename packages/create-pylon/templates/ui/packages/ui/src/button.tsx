import * as React from "react";
import { cn } from "./cn";

type Variant = "default" | "primary" | "ghost";
type Size = "sm" | "md";

const variants: Record<Variant, string> = {
	default:
		"bg-neutral-100 hover:bg-neutral-200 text-neutral-900 dark:bg-neutral-800 dark:hover:bg-neutral-700 dark:text-neutral-100",
	primary:
		"bg-neutral-900 hover:bg-neutral-800 text-white dark:bg-white dark:hover:bg-neutral-200 dark:text-neutral-900",
	ghost:
		"bg-transparent hover:bg-neutral-100 text-neutral-700 dark:hover:bg-neutral-800 dark:text-neutral-300",
};

const sizes: Record<Size, string> = {
	sm: "h-8 px-3 text-[13px]",
	md: "h-9 px-4 text-sm",
};

export interface ButtonProps
	extends React.ButtonHTMLAttributes<HTMLButtonElement> {
	variant?: Variant;
	size?: Size;
}

export function Button({
	className,
	variant = "default",
	size = "md",
	...props
}: ButtonProps) {
	return (
		<button
			className={cn(
				"inline-flex items-center justify-center gap-1.5 rounded-md font-medium transition-colors disabled:opacity-50 disabled:pointer-events-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500",
				variants[variant],
				sizes[size],
				className,
			)}
			{...props}
		/>
	);
}
