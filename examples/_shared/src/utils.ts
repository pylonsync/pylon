import type * as React from "react";
import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/**
 * Props of a ui-sdk part with `className` limited to a string. A part accepts
 * a function of its state as `className`; these wrappers merge class names with
 * `cn`, which takes strings.
 */
export type StyledProps<T extends React.ElementType> = Omit<
  React.ComponentProps<T>,
  "className"
> & { className?: string };
