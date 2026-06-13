import { type ClassValue, clsx } from "clsx";
import { twMerge } from "tailwind-merge";

// `cn` — the shadcn class merger. clsx resolves conditional/array class
// inputs; tailwind-merge then dedupes conflicting Tailwind utilities so
// the last one wins (e.g. `cn("px-2", "px-4")` → "px-4"). Every shadcn
// component routes its className through this.
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
