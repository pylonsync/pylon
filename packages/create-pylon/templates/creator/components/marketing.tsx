import React from "react";

// Presentational pieces for the marketing pages. All server-rendered, no client
// JS. Colors come from CSS vars set on <html> in app/layout.tsx, which read
// lib/site.config.ts, so re-theming is one edit there.

// One narrow column, left aligned, like a well-set essay page.
export const WRAP = "mx-auto w-full max-w-[40rem] px-6";

// A rule between sections.
export function Divider() {
  return (
    <div className={WRAP}>
      <hr className="border-0 border-t border-rule" />
    </div>
  );
}

// Section heading. One size, one weight, no label above it.
export function SectionTitle({ children }: { children: React.ReactNode }) {
  return <h2 className="text-[1.5rem] font-medium leading-[1.2] text-ink">{children}</h2>;
}

// Running text: 17.5px with generous leading, the same serif as the headings.
export const PROSE = "text-[17.5px] leading-[1.65] text-ink-2";

// A deliberately-obvious image placeholder: dashed border, photo glyph, and a
// "swap this" hint. Real sites drop a photo here.
//
//   shape  — "landscape" | "portrait" | "square" | "circle"
//   title  — what photo belongs here ("Your headshot")
//   hint   — how to replace it ("Replace in app/page.tsx")
export function ImagePlaceholder({
  shape = "landscape",
  title,
  hint,
  src,
  className = "",
}: {
  shape?: "landscape" | "portrait" | "square" | "circle";
  title: string;
  hint?: string;
  /** A real image. The template ships one under public/images; swap it for yours. */
  src?: string;
  className?: string;
}) {
  const aspect =
    shape === "portrait"
      ? "aspect-[4/5]"
      : shape === "square" || shape === "circle"
        ? "aspect-square"
        : "aspect-[4/3]";
  const radius = shape === "circle" ? "rounded-full" : "rounded-lg";
  if (src) {
    return (
      <div className={`relative overflow-hidden bg-rule ${aspect} ${radius} ${className}`}>
        <img src={src} alt={title} className="absolute inset-0 size-full object-cover" />
      </div>
    );
  }
  return (
    <div
      className={`relative grid place-items-center overflow-hidden border-2 border-dashed border-rule bg-paper ${aspect} ${radius} ${className}`}
    >
      <div className="px-4 text-center">
        <svg
          className="mx-auto size-7 text-rule"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden
        >
          <rect x="3" y="3" width="18" height="18" rx="2" />
          <circle cx="9" cy="9" r="1.6" />
          <path d="m21 15-4.5-4.5L7 20" />
        </svg>
        <p className="mt-2 text-[12.5px] font-medium text-ink-2">{title}</p>
        {hint ? <p className="mt-1 text-[11px] leading-snug text-ink-2/70">{hint}</p> : null}
      </div>
    </div>
  );
}
