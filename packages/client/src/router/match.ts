"use client";

import type { ReactNode } from "react";

export interface RouteSpec {
	/** Path pattern. `/users/:id`, `/posts/:slug?`, `/*` for catch-all. */
	path: string;
	/** Component rendered when matched. */
	component?: () => ReactNode;
	/** Explicit JSX alternative when you don't want a factory. */
	element?: ReactNode;
	/**
	 * Require an authenticated session. `true` = signed in. `"admin"` =
	 * signed in AND isAdmin. Custom predicates: render your own gate
	 * inside the component instead.
	 */
	requireAuth?: boolean | "admin";
	/** Where to send unauthenticated visitors. Default: `/sign-in`. */
	signInUrl?: string;
	/** Nested routes — parent renders `<Outlet />` where these appear. */
	children?: RouteSpec[];
}

export interface MatchedRoute {
	route: RouteSpec;
	params: Record<string, string>;
	/** Tail of the path that wasn't consumed by this route — used to
	 *  match children. Empty string when the parent consumed the whole
	 *  path. */
	tail: string;
}

/**
 * Walk the routes tree depth-first, picking the first leaf that
 * fully consumes the pathname. Returns the full match chain
 * (root → ... → leaf) so layouts can render via `<Outlet />`.
 */
export function matchRoutes(
	routes: RouteSpec[],
	pathname: string,
): MatchedRoute[] | null {
	const normalized = normalize(pathname);
	for (const route of routes) {
		const here = matchOne(route, normalized);
		if (!here) continue;
		if (route.children && route.children.length > 0) {
			const childChain = matchRoutes(route.children, here.tail);
			if (childChain) return [here, ...childChain];
			// A parent with children must match a child too; otherwise
			// we treat the parent as non-matching for this URL. This
			// avoids rendering a layout with no page inside it.
			continue;
		}
		// Leaf — require the tail to be empty so we don't half-match.
		if (here.tail === "" || here.tail === "/") return [here];
	}
	return null;
}

function matchOne(route: RouteSpec, pathname: string): MatchedRoute | null {
	const pattern = normalize(route.path);
	const isCatchAll = pattern.endsWith("/*");
	const patternSegments = pattern
		.replace(/\/\*$/, "")
		.split("/")
		.filter(Boolean);
	const pathSegments = pathname.split("/").filter(Boolean);

	const params: Record<string, string> = {};
	let i = 0;
	for (; i < patternSegments.length; i++) {
		const p = patternSegments[i]!;
		const seg = pathSegments[i];
		if (p.startsWith(":")) {
			const optional = p.endsWith("?");
			const name = p.slice(1).replace(/\?$/, "");
			if (seg === undefined) {
				if (optional) continue;
				return null;
			}
			params[name] = decodeURIComponent(seg);
		} else {
			if (seg === undefined || seg !== p) return null;
		}
	}

	if (isCatchAll) {
		const rest = pathSegments.slice(i).join("/");
		params["*"] = rest;
		return { route, params, tail: "" };
	}

	const tail = "/" + pathSegments.slice(i).join("/");
	// When children are declared we leave the tail for them; otherwise
	// the leaf check above requires tail === "".
	return { route, params, tail: tail === "/" ? "" : tail };
}

function normalize(p: string): string {
	if (!p) return "/";
	const trimmed = p.replace(/\/+$/, "");
	if (!trimmed.startsWith("/")) return `/${trimmed}`;
	return trimmed === "" ? "/" : trimmed;
}

export { normalize };
