import React from "react";

// Server-rendered layout helpers for the storefront pages. Restyle here and
// every page follows. The brand accent (`text-brand`, `bg-brand`) comes from
// CSS vars set on <html> in app/layout.tsx, which read lib/site.config.ts.

// Shared container. The storefront runs wide so product photos stay large.
export const WRAP = "mx-auto w-full max-w-[1360px] px-5 sm:px-8 lg:px-12";

// Pill buttons. `BUTTON_DARK` is the primary action; `BUTTON_LINE` sits beside it.
export const BUTTON_DARK =
  "inline-flex h-12 items-center justify-center gap-2 rounded-full bg-ink px-7 text-[14.5px] font-medium text-warm-white transition-[background-color,transform] duration-300 ease-out-quint hover:bg-brand active:scale-[0.98]";
export const BUTTON_LINE =
  "inline-flex h-12 items-center justify-center gap-2 rounded-full border border-ink/25 px-7 text-[14.5px] font-medium text-ink transition-[background-color,border-color,transform] duration-300 ease-out-quint hover:border-ink hover:bg-ink/[0.04] active:scale-[0.98]";

// A section heading in the display serif.
export function SectionHeading({ id, children }: { id?: string; children: React.ReactNode }) {
  return (
    <h2
      id={id}
      className="font-display text-[2.5rem] font-light leading-[1.02] tracking-[-0.02em] text-balance sm:text-[3.5rem]"
    >
      {children}
    </h2>
  );
}
