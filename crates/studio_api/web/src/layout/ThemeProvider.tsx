import { useEffect } from "react";
import { Preferences } from "@ui-sdk/react/preferences";
import type { ThemeAccent, ThemeConfig } from "@/lib/studio-config";
import { colorScheme } from "@/lib/theme";

const ACCENTS: ThemeAccent[] = ["emerald", "blue", "violet", "rose", "amber"];

/**
 * Applies the theme to <html>.
 *
 * The color scheme (`data-color-scheme`) is the operator's choice. The accent
 * (`data-accent`) and the optional custom primary color come from
 * studio.config.ts. A custom primary color is written as an inline style, so
 * it wins over the accent. Its text color is not derived from it: a very
 * light custom color needs its own `--primary-foreground`.
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

	useEffect(() => {
		const root = document.documentElement;
		root.setAttribute("data-accent", accent);
		for (const name of ["--primary", "--ring", "--link"]) {
			if (theme?.primary) root.style.setProperty(name, theme.primary);
			else root.style.removeProperty(name);
		}
	}, [accent, theme?.primary]);

	return <Preferences preferences={[colorScheme]}>{children}</Preferences>;
}
