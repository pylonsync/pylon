"use client";
import React from "react";

import { type ReactNode, useEffect } from "react";
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

export interface HasRoleProps {
	/** Role string (e.g., "owner", "admin", "member"). Matches against the
	 *  `roles` array on the resolved session, which the framework populates
	 *  with the user's org role when an org is active. */
	role: string | string[];
	/** Match ANY (default) or ALL listed roles. */
	mode?: "any" | "all";
	children: ReactNode;
	fallback?: ReactNode;
}

/**
 * Render children only when the resolved session has one (or all) of the
 * given roles. Server-side authorization is still the source of truth —
 * this is purely a UX gate to hide buttons the API would refuse anyway.
 */
export function HasRole({
	role,
	mode = "any",
	children,
	fallback = null,
}: HasRoleProps) {
	const { session } = useAuth();
	const roles = session?.roles ?? [];
	const wanted = Array.isArray(role) ? role : [role];
	const allowed =
		mode === "all"
			? wanted.every((r) => roles.includes(r))
			: wanted.some((r) => roles.includes(r));
	return allowed ? <>{children}</> : <>{fallback}</>;
}

/** Render children only when the user has an active org selected. */
export function InOrg({
	children,
	fallback = null,
}: {
	children: ReactNode;
	fallback?: ReactNode;
}) {
	const { tenantId } = useAuth();
	return tenantId ? <>{children}</> : <>{fallback}</>;
}

/** Render children only when no org is selected (personal mode). */
export function NoOrg({
	children,
	fallback = null,
}: {
	children: ReactNode;
	fallback?: ReactNode;
}) {
	const { tenantId } = useAuth();
	return tenantId ? <>{fallback}</> : <>{children}</>;
}

export interface RedirectToSignInProps {
	/** Sign-in route (default: "/sign-in"). */
	signInUrl?: string;
	/** Encode the current URL into a `next` param so the sign-in page can
	 *  bounce the user back. Default: true. */
	preserveLocation?: boolean;
}

/**
 * Imperative client-side redirect — drop inside `<SignedOut>` to send an
 * anonymous visitor to the sign-in page. Uses `window.location.assign`
 * (full nav) since we don't assume the consumer's router shape; apps
 * that want a soft navigation should write their own using `useAuth`.
 */
export function RedirectToSignIn({
	signInUrl = "/sign-in",
	preserveLocation = true,
}: RedirectToSignInProps) {
	useEffect(() => {
		if (typeof window === "undefined") return;
		const next =
			preserveLocation && window.location?.href
				? `?next=${encodeURIComponent(window.location.href)}`
				: "";
		window.location.assign(`${signInUrl}${next}`);
	}, [signInUrl, preserveLocation]);
	return null;
}
