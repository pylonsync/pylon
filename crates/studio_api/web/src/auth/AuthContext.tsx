import {
	createContext,
	useCallback,
	useContext,
	useEffect,
	useMemo,
	useState,
} from "react";
import { type AuthMe, type StudioUser, api } from "@/lib/pylon";

// Studio authenticates as a person, using whatever session the app itself
// issued — the same cookie the rest of the app runs on. There is no Studio
// credential to obtain, paste, or store.
//
// The server already refused to serve this bundle to anyone who isn't a
// signed-in admin (see `studio_access` in crates/runtime/src/server.rs), so by
// the time this provider runs the answer is known to be "an admin". We still
// resolve it client-side, for two reasons: the UI shows *who* you are, and a
// session that expires while the tab is open should surface as a locked state
// rather than a page of failing fetches.
//
// `/api/auth/session` returns the auth context AND the User row in one
// round-trip, so the sidebar can show an email without a second request.

type AuthState = {
	/** Resolved auth context. `null` while the first request is in flight. */
	me: AuthMe | null;
	/** The signed-in User row, minus server-only fields. `null` if anonymous. */
	user: StudioUser | null;
	loading: boolean;
	refresh: () => Promise<void>;
};

type SessionResponse = {
	session?: AuthMe | null;
	user?: StudioUser | null;
};

const ANONYMOUS: AuthMe = { user_id: null, is_admin: false, roles: [] };

const AuthCtx = createContext<AuthState | null>(null);

export function AuthProvider({ children }: { children: React.ReactNode }) {
	const [me, setMe] = useState<AuthMe | null>(null);
	const [user, setUser] = useState<StudioUser | null>(null);
	const [loading, setLoading] = useState(true);

	const refresh = useCallback(async () => {
		setLoading(true);
		try {
			const resp = await api<SessionResponse>("/api/auth/session");
			setMe(resp?.session ?? ANONYMOUS);
			setUser(resp?.user ?? null);
		} catch {
			// A failure here means the session went away underneath us (or the
			// network did). Either way the honest answer is "not signed in" —
			// claiming otherwise just produces a UI whose every panel errors.
			setMe(ANONYMOUS);
			setUser(null);
		} finally {
			setLoading(false);
		}
	}, []);

	useEffect(() => {
		void refresh();
	}, [refresh]);

	const value = useMemo<AuthState>(
		() => ({ me, user, loading, refresh }),
		[me, user, loading, refresh],
	);

	return <AuthCtx.Provider value={value}>{children}</AuthCtx.Provider>;
}

export function useAuth(): AuthState {
	const ctx = useContext(AuthCtx);
	if (!ctx) throw new Error("useAuth must be used inside <AuthProvider>");
	return ctx;
}

/** Display name for the signed-in admin. Falls back through the usual fields. */
export function displayName(user: StudioUser | null, me: AuthMe | null): string {
	if (user) {
		for (const key of ["name", "email", "username"] as const) {
			const v = user[key];
			if (typeof v === "string" && v.trim()) return v.trim();
		}
	}
	return me?.user_id ?? "Signed in";
}
