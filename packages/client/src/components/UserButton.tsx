"use client";

import {
	type ReactNode,
	useEffect,
	useRef,
	useState,
} from "react";
import { useAuth } from "../hooks/useAuth";
import { cn } from "../lib/cn";

export interface UserButtonProps {
	/** Where to send the user after sign-out. Default: current page reload. */
	afterSignOutUrl?: string;
	/** Show the user's name next to the avatar (Clerk's `showName`). */
	showName?: boolean;
	/** Extra menu items rendered above the built-in entries. */
	menuItems?: Array<{
		label: ReactNode;
		onClick?: () => void;
		href?: string;
	}>;
	className?: string;
}

/**
 * Avatar dropdown with profile/sign-out, mirroring Clerk's `<UserButton />`.
 * Renders nothing when the user isn't signed in — wrap the call site in
 * `<SignedIn>` or check `useAuth()` if you want a sign-in CTA instead.
 */
export function UserButton({
	afterSignOutUrl,
	showName,
	menuItems,
	className,
}: UserButtonProps) {
	const { isSignedIn, userId, session, signOut } = useAuth();
	const [open, setOpen] = useState(false);
	const rootRef = useRef<HTMLDivElement | null>(null);

	useEffect(() => {
		if (!open) return;
		function onDocClick(e: MouseEvent) {
			if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
		}
		function onKey(e: KeyboardEvent) {
			if (e.key === "Escape") setOpen(false);
		}
		document.addEventListener("mousedown", onDocClick);
		document.addEventListener("keydown", onKey);
		return () => {
			document.removeEventListener("mousedown", onDocClick);
			document.removeEventListener("keydown", onKey);
		};
	}, [open]);

	if (!isSignedIn) return null;

	// ResolvedSession is intentionally minimal (userId / tenantId / roles).
	// Apps that want a real name / email shown on the avatar pass it in
	// via `menuItems` or wrap their own component over the User entity.
	const label = userId ?? "Account";
	const initials = initialsFor(label);
	// session reserved for future extension hooks
	void session;

	async function onSignOut() {
		setOpen(false);
		await signOut();
		if (afterSignOutUrl && typeof window !== "undefined") {
			window.location.assign(afterSignOutUrl);
		}
	}

	return (
		<div ref={rootRef} className={cn("relative inline-flex", className)}>
			<button
				type="button"
				onClick={() => setOpen((o) => !o)}
				aria-haspopup="menu"
				aria-expanded={open}
				className="inline-flex items-center gap-2 rounded-full focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--pylon-ink,#0a0a0a)]"
			>
				<span
					className="flex h-8 w-8 items-center justify-center rounded-full bg-[var(--pylon-ink,#0a0a0a)] text-xs font-semibold text-[var(--pylon-paper,#ffffff)]"
					aria-hidden
				>
					{initials}
				</span>
				{showName ? (
					<span className="text-sm font-medium text-[var(--pylon-ink,#0a0a0a)]">
						{label}
					</span>
				) : null}
			</button>
			{open ? (
				<div
					role="menu"
					className="absolute right-0 top-full z-50 mt-2 w-56 overflow-hidden rounded-lg border border-[var(--pylon-rule,#e5e7eb)] bg-[var(--pylon-paper,#ffffff)] shadow-lg"
				>
					<div className="border-b border-[var(--pylon-rule,#e5e7eb)] px-3 py-2.5">
						<p className="truncate text-sm font-medium text-[var(--pylon-ink,#0a0a0a)]">
							{label}
						</p>
						{userId ? (
							<p className="truncate text-xs text-[var(--pylon-ink-3,#71717a)]">
								{userId}
							</p>
						) : null}
					</div>
					<div className="py-1">
						{(menuItems ?? []).map((item, i) => (
							<MenuItem
								key={i}
								onClick={() => {
									item.onClick?.();
									setOpen(false);
								}}
								href={item.href}
							>
								{item.label}
							</MenuItem>
						))}
						<MenuItem onClick={onSignOut}>Sign out</MenuItem>
					</div>
				</div>
			) : null}
		</div>
	);
}

function MenuItem({
	onClick,
	href,
	children,
}: {
	onClick?: () => void;
	href?: string;
	children: ReactNode;
}) {
	const cls =
		"block w-full px-3 py-2 text-left text-sm text-[var(--pylon-ink,#0a0a0a)] transition-colors hover:bg-[var(--pylon-paper-2,#f4f4f5)]";
	if (href) {
		return (
			<a href={href} role="menuitem" className={cls} onClick={onClick}>
				{children}
			</a>
		);
	}
	return (
		<button type="button" role="menuitem" onClick={onClick} className={cls}>
			{children}
		</button>
	);
}

function initialsFor(label: string): string {
	const parts = label.split(/\s+/).filter(Boolean);
	if (parts.length >= 2) return (parts[0]![0]! + parts[1]![0]!).toUpperCase();
	const seed = parts[0] ?? label;
	return seed.slice(0, 2).toUpperCase();
}
