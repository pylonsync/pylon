// The color scheme of Studio: system, light or dark.
//
// The choice is saved in localStorage and written to `data-color-scheme` on
// <html>. `theme.appearance` from studio.config.ts is the scheme an operator
// starts with.

import { createColorScheme, createLocalStorage } from "@ui-sdk/core";
import { resolveConfig } from "@/lib/studio-config";

export const COLOR_SCHEMES = ["system", "light", "dark"] as const;
export type ColorSchemeChoice = (typeof COLOR_SCHEMES)[number];

export const colorScheme = createColorScheme({
	storage: createLocalStorage("pylon-studio-color-scheme"),
	defaultValue: resolveConfig().theme?.appearance ?? "system",
	global: true,
});
