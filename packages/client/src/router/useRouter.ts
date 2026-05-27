"use client";

import { useContext } from "react";
import { RouterContext, type RouterContextValue } from "./context";

/**
 * Access the router from any descendant of `<Router />`. Returns a
 * stable snapshot of the current URL plus `push` / `replace` / `back`
 * helpers. Throws when used outside a `<Router />` — that's almost
 * always a bug worth surfacing immediately.
 *
 * ```tsx
 * const { pathname, params, push } = useRouter();
 * useEffect(() => { if (!user) push("/sign-in"); }, [user]);
 * ```
 */
export function useRouter(): RouterContextValue {
	const ctx = useContext(RouterContext);
	if (!ctx) {
		throw new Error(
			"[@pylonsync/client] useRouter() must be called inside a <Router /> tree",
		);
	}
	return ctx;
}

/** Shorthand for `useRouter().params`. */
export function useParams<T extends Record<string, string>>(): T {
	return useRouter().params as T;
}

/** Shorthand for `useRouter().query`. */
export function useSearchParams(): Record<string, string> {
	return useRouter().query;
}

/** Shorthand for `useRouter().pathname`. */
export function usePathname(): string {
	return useRouter().pathname;
}
