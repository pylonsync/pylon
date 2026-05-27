"use client";

import { type ReactNode } from "react";
import { useAuth } from "../hooks/useAuth";
import { cn } from "../lib/cn";

export interface SignOutButtonProps {
	afterSignOutUrl?: string;
	className?: string;
	children?: ReactNode;
}

/**
 * Headless-friendly sign-out button. Pass children to fully customize
 * the rendered label/UI; default renders a plain "Sign out" button.
 */
export function SignOutButton({
	afterSignOutUrl,
	className,
	children,
}: SignOutButtonProps) {
	const { signOut } = useAuth();
	async function onClick() {
		await signOut();
		if (afterSignOutUrl && typeof window !== "undefined") {
			window.location.assign(afterSignOutUrl);
		}
	}
	return (
		<button
			type="button"
			onClick={onClick}
			className={cn(
				"inline-flex items-center justify-center rounded-md px-3 py-1.5 text-sm font-medium text-[var(--pylon-ink-2,#52525b)] transition-colors hover:text-[var(--pylon-ink,#0a0a0a)]",
				className,
			)}
		>
			{children ?? "Sign out"}
		</button>
	);
}
