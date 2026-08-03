// Studio's URL scheme.
//
// v1 kept the current page in a `useState` and never touched the URL. Every
// consequence of that was felt constantly: you couldn't link a colleague to a
// table, a refresh dumped you back on Overview, and the back button left
// Studio entirely rather than going back one page.
//
// The scheme:
//
//   /studio                 the default page (Overview, or the first page the
//                           app's sidebar config declares)
//   /studio/<pageId>        a built-in or extension page
//   /studio/e/<entity>      an entity's list
//
// `e/` prefixes entities so a page id can never collide with an entity name —
// an app with an entity called `settings` would otherwise shadow the Settings
// page, and which one won would depend on match order.
//
// Studio is always mounted at /studio (the Rust handler hard-codes it), so the
// base is a constant rather than something to discover at runtime.

import type { StudioRoute } from "@/layout/StudioLayout";

export const STUDIO_BASE = "/studio";

/** Page shown for a bare `/studio` when the app declares no first page. */
export const DEFAULT_PAGE = "overview";

/**
 * URL path for a route. Entity names and page ids are encoded — an entity is
 * a manifest identifier, but extension page ids are app-authored strings and
 * there is no reason to trust them into a URL unescaped.
 */
export function routeToPath(route: StudioRoute): string {
	if (route.kind === "resource") {
		return `${STUDIO_BASE}/e/${encodeURIComponent(route.entity)}`;
	}
	return `${STUDIO_BASE}/${encodeURIComponent(route.id)}`;
}

/**
 * Parse a pathname into a route. Returns null when the path isn't a Studio
 * route at all, so the caller can fall back to its configured default rather
 * than guess.
 *
 * Tolerates a trailing slash and any `?query` / `#hash` the caller left on.
 */
export function parseRoute(pathname: string): StudioRoute | null {
	const path = pathname.split("?")[0].split("#")[0];
	if (path !== STUDIO_BASE && !path.startsWith(`${STUDIO_BASE}/`)) return null;

	const rest = path.slice(STUDIO_BASE.length).replace(/^\/+|\/+$/g, "");
	if (!rest) return null; // bare /studio — caller decides the default

	const segments = rest.split("/").map(decodeSegment);

	if (segments[0] === "e") {
		const entity = segments[1];
		// `/studio/e` with no entity is not a route. Falling back to the
		// default beats rendering a list for the empty-string entity.
		return entity ? { kind: "resource", entity } : null;
	}

	// Extra segments are ignored rather than rejected: `/studio/overview/x`
	// should land on Overview, not a 404 inside a single-page shell.
	return { kind: "page", id: segments[0] };
}

function decodeSegment(s: string): string {
	try {
		return decodeURIComponent(s);
	} catch {
		// A malformed `%` sequence throws. Use the raw segment — it won't match
		// a page or entity, so it renders the "unknown page" card, which is a
		// better answer than a blank screen from an uncaught URIError.
		return s;
	}
}

/** Do two routes address the same page? Used to skip redundant history entries. */
export function sameRoute(a: StudioRoute, b: StudioRoute): boolean {
	if (a.kind !== b.kind) return false;
	return a.kind === "resource" && b.kind === "resource"
		? a.entity === b.entity
		: a.kind === "page" && b.kind === "page" && a.id === b.id;
}
