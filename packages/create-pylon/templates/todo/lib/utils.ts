import { type ClassValue, clsx } from "clsx";
import { twMerge } from "tailwind-merge";

// `cn` merges class names. clsx resolves conditional and array inputs, then
// tailwind-merge removes conflicting Tailwind utilities so the last one wins:
// `cn("px-2", "px-4")` is "px-4". Every component passes its `className`
// through this, so a caller can override a default style.
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
