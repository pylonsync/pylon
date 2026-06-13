"use client";
import React from "react";

import {
	type AnchorHTMLAttributes,
	type ReactNode,
	useCallback,
	useContext,
	useEffect,
	useMemo,
	useState,
} from "react";
import { useAuth } from "../hooks/useAuth";
import { RouterContext, RouterDepthContext } from "./context";
import { matchRoutes, type MatchedRoute, type RouteSpec } from "./match";

export interface RouterProps {
	routes: RouteSpec[];
	/** Render when no route matches. Default: a minimal 404 line. */
	notFound?: ReactNode;
	/** Initial pathname for SSR / testing. Default: window.location.pathname. */
	initialUrl?: string;
}

/**
 * Top-level SPA router. Subscribes to `popstate` + intercepted clicks
 * from `<Link />`, matches the URL against the `routes` tree, and
 * renders the matched chain with nested `<Outlet />` support.
 *
 * Place once near the root of the app:
 *
 * ```tsx
 * <Router routes={[
 *   { path: "/", component: HomePage },
 *   {
 *     path: "/dashboard",
 *     component: DashboardLayout,
 *     requireAuth: true,
 *     children: [
 *       { path: "/", component: DashboardHome },
 *       { path: "/settings", component: SettingsPage },
 *     ],
 *   },
 * ]} />
 * ```
 */
export function Router({ routes, notFound, initialUrl }: RouterProps) {
	const [url, setUrl] = useState<URL>(() => initialURL(initialUrl));

	useEffect(() => {
		if (typeof window === "undefined") return;
		// Pick up the live URL on mount in case the SSR-painted snapshot
		// is stale (hashchange, etc).
		setUrl(new URL(window.location.href));
		function onPop() {
			setUrl(new URL(window.location.href));
		}
		window.addEventListener("popstate", onPop);
		return () => window.removeEventListener("popstate", onPop);
	}, []);

	const navigate = useCallback(
		(href: string, mode: "push" | "replace") => {
			if (typeof window === "undefined") return;
			const next = new URL(href, window.location.origin);
			if (mode === "push") {
				window.history.pushState({}, "", next.href);
			} else {
				window.history.replaceState({}, "", next.href);
			}
			setUrl(next);
			// Scroll to top on push, matching what users expect from a
			// SPA navigation. Replace skips this — usually a side-effect
			// of in-place state (e.g., filter toggles).
			if (mode === "push") {
				window.scrollTo(0, 0);
			}
		},
		[],
	);

	const chain = useMemo(
		() => matchRoutes(routes, url.pathname),
		[routes, url.pathname],
	);

	const params = useMemo(() => {
		const merged: Record<string, string> = {};
		for (const m of chain ?? []) Object.assign(merged, m.params);
		return merged;
	}, [chain]);

	const query = useMemo(() => parseQuery(url.search), [url.search]);

	const value = useMemo(
		() => ({
			pathname: url.pathname,
			search: url.search,
			hash: url.hash,
			query,
			params,
			matches: chain ?? [],
			push: (href: string) => navigate(href, "push"),
			replace: (href: string) => navigate(href, "replace"),
			back: () => {
				if (typeof window !== "undefined") window.history.back();
			},
		}),
		[url, query, params, chain, navigate],
	);

	return (
		<RouterContext.Provider value={value}>
			<RouterDepthContext.Provider value={0}>
				{chain ? <RouteRenderer chain={chain} depth={0} /> : (notFound ?? <NotFound />)}
			</RouterDepthContext.Provider>
		</RouterContext.Provider>
	);
}

function RouteRenderer({
	chain,
	depth,
}: {
	chain: MatchedRoute[];
	depth: number;
}) {
	const m = chain[depth];
	if (!m) return null;
	const node = renderRoute(m.route);
	return (
		<RouterDepthContext.Provider value={depth}>
			<AuthGate route={m.route}>{node}</AuthGate>
		</RouterDepthContext.Provider>
	);
}

function AuthGate({
	route,
	children,
}: {
	route: RouteSpec;
	children: ReactNode;
}) {
	const { isSignedIn, isAdmin } = useAuth();
	const needsAdmin = route.requireAuth === "admin";
	const allowed = !route.requireAuth
		? true
		: needsAdmin
			? isSignedIn && isAdmin
			: isSignedIn;
	// Effect must be unconditional to honor rules of hooks. The redirect
	// only fires when both (a) the route is auth-gated and (b) the
	// caller isn't allowed.
	useEffect(() => {
		if (!route.requireAuth || allowed) return;
		if (typeof window === "undefined") return;
		const signInUrl = route.signInUrl ?? "/sign-in";
		const next = encodeURIComponent(
			window.location.pathname + window.location.search,
		);
		window.location.assign(`${signInUrl}?next=${next}`);
	}, [allowed, route.requireAuth, route.signInUrl]);
	return allowed ? <>{children}</> : null;
}

function renderRoute(route: RouteSpec): ReactNode {
	if (route.element !== undefined) return route.element;
	if (route.component) {
		const C = route.component;
		return <C />;
	}
	return null;
}

/**
 * Renders the next match in the chain — the page nested under the
 * current layout. Without an `<Outlet />` a layout component's
 * children never appear.
 */
export function Outlet() {
	const router = useContext(RouterContext);
	const depth = useContext(RouterDepthContext);
	if (!router) return null;
	if (depth + 1 >= router.matches.length) return null;
	return <RouteRenderer chain={router.matches} depth={depth + 1} />;
}

export interface LinkProps extends AnchorHTMLAttributes<HTMLAnchorElement> {
	href: string;
	/** Use replaceState instead of pushState (no history entry added). */
	replace?: boolean;
}

/**
 * SPA link. Renders a real `<a>` so middle-click / cmd-click / right-
 * click → "open in new tab" all behave correctly. Plain left-click is
 * intercepted and routed via the in-page Router.
 */
export function Link({
	href,
	replace,
	onClick,
	target,
	rel,
	children,
	...rest
}: LinkProps) {
	const router = useContext(RouterContext);
	function handleClick(e: React.MouseEvent<HTMLAnchorElement>) {
		onClick?.(e);
		if (e.defaultPrevented) return;
		if (
			e.button !== 0 ||
			e.metaKey ||
			e.ctrlKey ||
			e.shiftKey ||
			e.altKey ||
			target === "_blank"
		)
			return;
		if (!router) return;
		// Only intercept same-origin relative paths.
		try {
			const u = new URL(href, window.location.origin);
			if (u.origin !== window.location.origin) return;
			e.preventDefault();
			if (replace) router.replace(u.pathname + u.search + u.hash);
			else router.push(u.pathname + u.search + u.hash);
		} catch {
			/* malformed href — let the browser handle it */
		}
	}
	return (
		<a
			href={href}
			target={target}
			rel={target === "_blank" ? (rel ?? "noopener noreferrer") : rel}
			onClick={handleClick}
			{...rest}
		>
			{children}
		</a>
	);
}

function NotFound() {
	return (
		<div className="mx-auto max-w-md px-6 py-16 text-center">
			<p className="text-4xl font-semibold tracking-tight text-[var(--pylon-ink,#0a0a0a)]">
				404
			</p>
			<p className="mt-2 text-sm text-[var(--pylon-ink-2,#52525b)]">
				Nothing here.
			</p>
		</div>
	);
}

function initialURL(initialUrl?: string): URL {
	if (initialUrl) {
		try {
			return new URL(initialUrl, "http://localhost");
		} catch {
			/* fall through */
		}
	}
	if (typeof window !== "undefined") {
		return new URL(window.location.href);
	}
	return new URL("http://localhost/");
}

function parseQuery(search: string): Record<string, string> {
	const out: Record<string, string> = {};
	if (!search) return out;
	const params = new URLSearchParams(
		search.startsWith("?") ? search.slice(1) : search,
	);
	for (const [k, v] of params.entries()) out[k] = v;
	return out;
}
