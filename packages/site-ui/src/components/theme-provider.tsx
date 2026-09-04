"use client";

import * as React from "react";
import { ThemeProvider as NextThemesProvider } from "next-themes";

// Thin wrapper around next-themes so we can swap the impl later without
// touching every consumer. Stack0 Cloud defaults to "system" — the user
// can toggle between paper / graphite via the header switcher.
export function ThemeProvider({
  children,
  ...props
}: React.ComponentProps<typeof NextThemesProvider>) {
  return <NextThemesProvider {...props}>{children}</NextThemesProvider>;
}
