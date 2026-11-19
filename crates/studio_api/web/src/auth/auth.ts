import { createContext, useContext } from "react";
import type { AuthMe, StudioUser } from "@/lib/pylon";

export type AuthState = {
	/** The auth context of the session. `null` while the first request runs. */
	me: AuthMe | null;
	/** The signed-in User row with no server-only fields. `null` if anonymous. */
	user: StudioUser | null;
	/** The username of the Studio operator, when the session is an operator session. */
	operator: string | null;
	loading: boolean;
	refresh: () => Promise<void>;
};

export const AuthCtx = createContext<AuthState | null>(null);

export function useAuth(): AuthState {
	const ctx = useContext(AuthCtx);
	if (!ctx) throw new Error("useAuth must be used inside <AuthProvider>");
	return ctx;
}

/** The name to show for the signed-in account. */
export function displayName({
	user,
	operator,
	me,
}: Pick<AuthState, "user" | "operator" | "me">): string {
	if (operator) return operator;
	if (user) {
		for (const key of ["name", "email", "username"] as const) {
			const v = user[key];
			if (typeof v === "string" && v.trim()) return v.trim();
		}
	}
	return me?.user_id ?? "Signed in";
}
