import {
	createContext,
	useCallback,
	useContext,
	useEffect,
	useMemo,
	useState,
} from "react";
import { type AuthMe, api, getStoredToken, setStoredToken } from "@/lib/pylon";

// Studio uses bearer-token auth (the operator pastes their PYLON_ADMIN_TOKEN
// into the login dialog). Cookie auth would also work for non-admin users
// but Studio is operator-facing — admin token is the primary path.
//
// `me` is the resolved AuthContext from /api/auth/me. `null` while
// resolving, then either the resolved user (admin or not) or an
// anonymous shape. UI tabs that need admin show a locked state when
// `me?.is_admin` is false.

type AuthState = {
	me: AuthMe | null;
	loading: boolean;
	hasToken: boolean;
	signIn: (token: string) => Promise<void>;
	signOut: () => void;
	refresh: () => Promise<void>;
};

const AuthCtx = createContext<AuthState | null>(null);

export function AuthProvider({ children }: { children: React.ReactNode }) {
	const [me, setMe] = useState<AuthMe | null>(null);
	const [loading, setLoading] = useState(true);
	const [token, setTokenState] = useState<string | null>(() => getStoredToken());

	const refresh = useCallback(async () => {
		setLoading(true);
		try {
			const resp = await api<AuthMe>("/api/auth/me");
			setMe(resp);
		} catch {
			setMe({ user_id: null, is_admin: false, roles: [] });
		} finally {
			setLoading(false);
		}
	}, []);

	useEffect(() => {
		void refresh();
		// `token` triggers re-resolve on sign-in/sign-out. The actual
		// token value is read via getStoredToken inside api().
	}, [refresh, token]);

	const signIn = useCallback(
		async (newToken: string) => {
			// Validate the token by hitting /api/auth/me with it. If it
			// resolves to is_admin=true we trust the operator and persist.
			// A user-token with is_admin=false is also accepted (Studio
			// is useful in read-only mode for non-admins; admin-gated
			// tabs will indicate they need elevation).
			const resp = await api<AuthMe>("/api/auth/me", { token: newToken });
			setStoredToken(newToken);
			setTokenState(newToken);
			setMe(resp);
		},
		[],
	);

	const signOut = useCallback(() => {
		setStoredToken(null);
		setTokenState(null);
		setMe({ user_id: null, is_admin: false, roles: [] });
	}, []);

	const value = useMemo<AuthState>(
		() => ({ me, loading, hasToken: !!token, signIn, signOut, refresh }),
		[me, loading, token, signIn, signOut, refresh],
	);

	return <AuthCtx.Provider value={value}>{children}</AuthCtx.Provider>;
}

export function useAuth(): AuthState {
	const ctx = useContext(AuthCtx);
	if (!ctx) throw new Error("useAuth must be used inside <AuthProvider>");
	return ctx;
}
