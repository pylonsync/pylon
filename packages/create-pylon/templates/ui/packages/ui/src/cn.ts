import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/**
 * Tailwind-aware class merger. Last-class-wins semantics so a
 * caller's `className` reliably overrides a default in a UI
 * primitive (e.g. <Button className="bg-red-500"> beats the
 * primitive's bg-neutral-900 base).
 */
export function cn(...inputs: ClassValue[]): string {
	return twMerge(clsx(inputs));
}
