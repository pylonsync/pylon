"use client";

import { db, useSession } from "@pylonsync/react";

/**
 * Drop-in hook that wraps `useSession(db.sync)` so components in
 * `@pylonsync/client` don't have to thread a SyncEngine through props.
 * Apps that init() the global engine (the common case) get a Clerk-style
 * `const { isSignedIn, user } = useAuth()` surface.
 */
export function useAuth() {
	const session = useSession(db.sync);
	return {
		isSignedIn: session.isAuthenticated,
		isLoaded: true,
		userId: session.userId,
		tenantId: session.tenantId,
		isAdmin: session.isAdmin,
		session: session.session,
		signOut: session.signOut,
		selectOrg: session.selectOrg,
		clearOrg: session.clearOrg,
		refresh: session.refresh,
	};
}

export type UseAuthReturn = ReturnType<typeof useAuth>;
