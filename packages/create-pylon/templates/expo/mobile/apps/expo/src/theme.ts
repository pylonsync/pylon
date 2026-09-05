import { useColorScheme } from "react-native";

export const palette = {
  light: {
    bg: "#ffffff",
    surface: "#f4f4f5",
    border: "#e4e4e7",
    text: "#18181b",
    muted: "#71717a",
    brand: "#18181b",
    onBrand: "#ffffff",
    danger: "#dc2626",
    success: "#16a34a",
  },
  dark: {
    bg: "#09090b",
    surface: "#18181b",
    border: "#27272a",
    text: "#fafafa",
    muted: "#a1a1aa",
    brand: "#fafafa",
    onBrand: "#09090b",
    danger: "#f87171",
    success: "#4ade80",
  },
} as const;

export type Theme = Record<keyof (typeof palette)["light"], string>;

export function useTheme(): Theme {
  const scheme = useColorScheme();
  return scheme === "dark" ? palette.dark : palette.light;
}

export const space = { xs: 4, sm: 8, md: 12, lg: 16, xl: 24, xxl: 32 } as const;
export const radius = { sm: 8, md: 12, lg: 16, pill: 999 } as const;
