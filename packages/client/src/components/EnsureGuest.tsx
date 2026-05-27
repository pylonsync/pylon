"use client";

import { type ReactNode, useEffect, useState } from "react";
import { ensureGuestSession } from "../lib/api";

export interface EnsureGuestProps {
	/**
	 * Render while the guest session is being minted. Defaults to
	 * nothing — the bootstrap is usually fast enough that a flash is
	 * worse than empty space. Pass a spinner / skeleton if you care.
	 */
	fallback?: ReactNode;
	children: ReactNode;
}

/**
 * Drop-in wrapper that ensures the browser has a Pylon session before
 * rendering children. If a token is already cached, renders
 * immediately; otherwise calls `POST /api/auth/guest` and renders
 * once the session token lands.
 *
 * Use this for zero-auth demos where every visitor implicitly becomes
 * their own user — recordings, todos, scratch surfaces. Apps with
 * real users should pair `<SignedOut><SignIn /></SignedOut>` with
 * `<SignedIn>` instead.
 *
 * ```tsx
 * <EnsureGuest>
 *   <TodoList />
 * </EnsureGuest>
 * ```
 */
export function EnsureGuest({ fallback = null, children }: EnsureGuestProps) {
	const [ready, setReady] = useState(false);

	useEffect(() => {
		let cancelled = false;
		void ensureGuestSession().finally(() => {
			if (!cancelled) setReady(true);
		});
		return () => {
			cancelled = true;
		};
	}, []);

	if (!ready) return <>{fallback}</>;
	return <>{children}</>;
}
