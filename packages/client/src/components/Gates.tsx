"use client";

import { type ReactNode } from "react";
import { useAuth } from "../hooks/useAuth";

/**
 * Render children only when the user is signed in. Drop-in match for
 * Clerk's `<SignedIn>`.
 */
export function SignedIn({ children }: { children: ReactNode }) {
	const { isSignedIn } = useAuth();
	return isSignedIn ? <>{children}</> : null;
}

/**
 * Render children only when the user is signed out.
 */
export function SignedOut({ children }: { children: ReactNode }) {
	const { isSignedIn } = useAuth();
	return isSignedIn ? null : <>{children}</>;
}

export interface ProtectProps {
	/** Show fallback (or null) when the predicate fails. */
	children: ReactNode;
	/** What to render when the user isn't authorized. Defaults to nothing. */
	fallback?: ReactNode;
	/** Require admin. Equivalent to passing `predicate={(a) => a.isAdmin}`. */
	admin?: boolean;
	/** Custom gate. Receives the full `useAuth()` shape. */
	predicate?: (auth: ReturnType<typeof useAuth>) => boolean;
}

/**
 * Gate children behind a predicate. Defaults to "must be signed in";
 * pass `admin` or a custom `predicate` for finer control. Server-side
 * authorization is still enforced by the policy layer — this is purely a
 * UX convenience to avoid flashing forbidden content.
 */
export function Protect({
	children,
	fallback = null,
	admin,
	predicate,
}: ProtectProps) {
	const auth = useAuth();
	const allowed = predicate
		? predicate(auth)
		: admin
			? auth.isSignedIn && auth.isAdmin
			: auth.isSignedIn;
	return allowed ? <>{children}</> : <>{fallback}</>;
}
