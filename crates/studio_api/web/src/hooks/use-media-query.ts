import { useSyncExternalStore } from "react";

/** Whether the CSS media query matches. The value follows the window. */
export function useMediaQuery(query: string): boolean {
	return useSyncExternalStore(
		(onChange) => {
			const mql = window.matchMedia(query);
			mql.addEventListener("change", onChange);
			return () => mql.removeEventListener("change", onChange);
		},
		() => window.matchMedia(query).matches,
	);
}

/** Under this width the sidebar is a sheet. It matches Tailwind's `md`. */
export const NARROW_QUERY = "(max-width: 767px)";
