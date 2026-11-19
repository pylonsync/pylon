// Navigation inside Studio without a page load.

import { routeToPath } from "@/lib/route";

/**
 * Go to a Studio URL. The history entry is pushed, then a `popstate` event
 * tells the app and the pages that read the query string to read the URL.
 */
export function pushUrl(url: string): void {
	window.history.pushState(null, "", url);
	window.dispatchEvent(new PopStateEvent("popstate"));
}

/** The URL of one row: the list of its entity with the row open. */
export function rowUrl(entity: string, id: string): string {
	return `${routeToPath({ kind: "resource", entity })}?row=${encodeURIComponent(id)}`;
}

/**
 * Click handler for an in-app link. A plain click navigates in place. A click
 * with a modifier key or a non-primary button keeps the browser behavior, so
 * the link opens in a new tab.
 */
export function onLinkClick(
	e: React.MouseEvent<HTMLAnchorElement>,
	url: string,
): void {
	if (e.defaultPrevented || e.button !== 0) return;
	if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
	e.preventDefault();
	pushUrl(url);
}
