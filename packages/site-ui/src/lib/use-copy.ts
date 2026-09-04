import { useState } from "react";

/**
 * Copy-to-clipboard with a self-clearing "copied" flag.
 *
 * Shared so the hero's install pill and the template cards behave the same:
 * both no-op when `navigator.clipboard` is unavailable (insecure origin, old
 * browser) rather than throwing an unhandled rejection into the console.
 */
export function useCopy(resetMs = 1600): {
	copied: boolean;
	copy: (text: string) => void;
} {
	const [copied, setCopied] = useState(false);

	function copy(text: string) {
		navigator.clipboard
			?.writeText(text)
			.then(() => {
				setCopied(true);
				setTimeout(() => setCopied(false), resetMs);
			})
			.catch(() => {
				/* clipboard unavailable — no-op */
			});
	}

	return { copied, copy };
}
