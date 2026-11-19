import { useMemo } from "react";

import { AuthCtx, type AuthState } from "@/auth/auth";
import { useQuery } from "@/hooks/use-query";
import { type AuthMe, type StudioUser, api } from "@/lib/pylon";

// Studio authenticates as a person. The session is a Studio operator session
// or the session of an app admin, and the cookie carries it. Studio holds no
// credential.
//
// The server serves Studio to a signed-in admin only, so the answer here is
// "an admin" on load. The provider still asks, for two reasons: the sidebar
// shows who is signed in, and a session that ends while the tab is open must
// show as a signed-out state.
//
// `/api/auth/session` returns the auth context and the User row in one
// request.

type SessionResponse = {
	session?: AuthMe | null;
	user?: StudioUser | null;
	operator?: { username?: string } | null;
};

const ANONYMOUS: AuthMe = { user_id: null, is_admin: false, roles: [] };

export function AuthProvider({ children }: { children: React.ReactNode }) {
	const { data, error, loading, reload } = useQuery("session", () =>
		api<SessionResponse>("/api/auth/session"),
	);

	const value = useMemo<AuthState>(() => {
		// A failed request means the session or the network is gone. Both
		// show as "not signed in".
		const failed = error !== null;
		return {
			me: failed ? ANONYMOUS : data ? (data.session ?? ANONYMOUS) : null,
			user: failed ? null : (data?.user ?? null),
			operator: failed ? null : (data?.operator?.username ?? null),
			loading: loading && !data && !failed,
			refresh: reload,
		};
	}, [data, error, loading, reload]);

	return <AuthCtx.Provider value={value}>{children}</AuthCtx.Provider>;
}
