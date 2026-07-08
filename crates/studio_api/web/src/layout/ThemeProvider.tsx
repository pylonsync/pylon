import { useEffect } from "react";
import { ThemeProvider as NextThemes } from "next-themes";
import type { ThemeAccent, ThemeConfig } from "@/lib/studio-config";

const ACCENTS: ThemeAccent[] = ["emerald", "blue", "violet", "rose", "amber"];

/**
 * Wraps `next-themes` and adds the accent-palette swap. Two
 * orthogonal axes: appearance (light/dark/system, handled by
 * next-themes via the `class` attribute on `<html>`) and accent
 * (one of five built-ins, applied via `data-accent` on `<html>`).
 *
 * `theme.primary` (custom hex) is applied as an inline style on
 * `<html>` that re-points `--primary` and `--ring`. Best-effort —
 * we don't recompute foreground contrast, so very light hexes
 * may need their own foreground override via CSS.
 */
export function ThemeProvider({
	theme,
	children,
}: {
	theme: ThemeConfig | undefined;
	children: React.ReactNode;
}) {
	const accent: ThemeAccent =
		theme?.accent && ACCENTS.includes(theme.accent) ? theme.accent : "blue";
	const defaultTheme = theme?.appearance ?? "light";

	useEffect(() => {
		if (typeof document === "undefined") return;
		const root = document.documentElement;
		root.setAttribute("data-accent", accent);

		// Custom primary override — re-point CSS vars inline so the
		// override survives accent class changes.
		if (theme?.primary) {
			root.style.setProperty("--primary", theme.primary);
			root.style.setProperty("--ring", theme.primary);
			root.style.setProperty("--sidebar-primary", theme.primary);
			root.style.setProperty("--sidebar-ring", theme.primary);
		} else {
			root.style.removeProperty("--primary");
			root.style.removeProperty("--ring");
			root.style.removeProperty("--sidebar-primary");
			root.style.removeProperty("--sidebar-ring");
		}
	}, [accent, theme?.primary]);

	return (
		<NextThemes
			attribute="class"
			defaultTheme={defaultTheme}
			enableSystem={defaultTheme === "system"}
			disableTransitionOnChange
		>
			{children}
		</NextThemes>
	);
}
