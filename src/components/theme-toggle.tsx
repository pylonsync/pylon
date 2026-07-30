"use client";

import * as React from "react";
import { Moon, Sun, Monitor } from "lucide-react";
import { useTheme } from "next-themes";
import { Button } from "./ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "./ui/dropdown-menu";

// 3-mode picker: paper / graphite / system. Defaults to system because
// the OS has the right answer most of the time. Avoid a flip-flop button
// — explicit choice reads better in a dev-tools dashboard.
export function ThemeToggle() {
  const { setTheme, theme } = useTheme();
  const [mounted, setMounted] = React.useState(false);
  React.useEffect(() => setMounted(true), []);

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon-sm" aria-label="Toggle theme">
          <Sun className="size-3.5 dark:hidden" />
          <Moon className="hidden size-3.5 dark:block" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem onClick={() => setTheme("light")}>
          <Sun /> Paper
          {mounted && theme === "light" && (
            <span className="ml-auto text-[var(--color-cobalt)]">·</span>
          )}
        </DropdownMenuItem>
        <DropdownMenuItem onClick={() => setTheme("dark")}>
          <Moon /> Graphite
          {mounted && theme === "dark" && (
            <span className="ml-auto text-[var(--color-cobalt)]">·</span>
          )}
        </DropdownMenuItem>
        <DropdownMenuItem onClick={() => setTheme("system")}>
          <Monitor /> System
          {mounted && theme === "system" && (
            <span className="ml-auto text-[var(--color-cobalt)]">·</span>
          )}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
