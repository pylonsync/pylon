import React from "react";

// Reusable presentational pieces for the landing page. All server-rendered —
// no client JS. Restyle here and the whole page follows. The brand accent
// (`text-brand`, `bg-brand-soft`) comes from CSS vars set on <html> in
// app/layout.tsx, which read lib/site.config.ts — so re-theming is one edit.

// Shared container: a contained, centered column.
export const WRAP = "mx-auto w-full max-w-5xl px-6";

export function Eyebrow({ children }: { children: React.ReactNode }) {
  return (
    <p className="font-mono text-[11px] font-semibold uppercase tracking-[0.14em] text-brand">
      {children}
    </p>
  );
}

// "New / Coming soon"-style pill for the hero.
export function Badge({ children }: { children: React.ReactNode }) {
  return (
    <span className="inline-flex items-center gap-2 rounded-full border border-zinc-200 bg-white py-1 pl-1.5 pr-3 text-[13px] text-zinc-600 shadow-sm">
      <span className="inline-block size-1.5 rounded-full bg-brand" />
      {children}
    </span>
  );
}

export function Divider() {
  return (
    <div className={WRAP}>
      <div className="border-t border-zinc-200/70" />
    </div>
  );
}

export function SectionHead({
  eyebrow,
  title,
  body,
}: {
  eyebrow: string;
  title: string;
  body?: string;
}) {
  return (
    <div>
      <Eyebrow>{eyebrow}</Eyebrow>
      <h2 className="mt-4 text-balance text-2xl font-semibold leading-[1.15] tracking-[-0.02em] sm:text-3xl">
        {title}
      </h2>
      {body ? (
        <p className="mt-4 max-w-xl text-[15px] leading-relaxed text-zinc-500">
          {body}
        </p>
      ) : null}
    </div>
  );
}

// A grid of value props — icon + title + body.
export function FeatureGrid({
  items,
}: {
  items: { title: string; body: string; icon?: string }[];
}) {
  return (
    <div className="grid gap-6 sm:grid-cols-3">
      {items.map((f) => (
        <div key={f.title}>
          {f.icon ? (
            <span className="flex size-9 items-center justify-center rounded-lg bg-brand-soft text-brand">
              {f.icon}
            </span>
          ) : null}
          <h3 className="mt-4 text-[15px] font-semibold text-zinc-900">
            {f.title}
          </h3>
          <p className="mt-2 text-[14px] leading-relaxed text-zinc-500">
            {f.body}
          </p>
        </div>
      ))}
    </div>
  );
}

// Initials for testimonial avatars, so the cards look finished without a photo.
export function initials(name: string) {
  return name
    .split(/\s+/)
    .map((w) => w[0])
    .join("")
    .slice(0, 2)
    .toUpperCase();
}

// A deliberately-obvious image placeholder. Real sites drop a photo here; this
// makes the spot unmistakable — dashed border, a photo glyph, and a one-line
// "swap this" instruction telling you exactly what to replace and where. Looks
// tidy enough to demo, but no one will mistake it for a finished design.
//
//   shape  — "landscape" | "portrait" | "square" | "circle"
//   title  — what photo belongs here ("A photo of your dining room")
//   hint   — how to replace it ("Swap for an <img> in app/(marketing)/page.tsx")
export function ImagePlaceholder({
  shape = "landscape",
  title,
  hint,
  className = "",
}: {
  shape?: "landscape" | "portrait" | "square" | "circle";
  title: string;
  hint?: string;
  className?: string;
}) {
  const aspect =
    shape === "portrait"
      ? "aspect-[4/5]"
      : shape === "square" || shape === "circle"
        ? "aspect-square"
        : "aspect-[4/3]";
  const radius = shape === "circle" ? "rounded-full" : "rounded-2xl";
  return (
    <div
      className={`relative grid place-items-center overflow-hidden border-2 border-dashed border-zinc-300 bg-zinc-50 ${aspect} ${radius} ${className}`}
    >
      <div className="px-6 text-center">
        <svg
          className="mx-auto size-7 text-zinc-300"
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
        <p className="mt-3 text-[13px] font-medium text-zinc-500">{title}</p>
        {hint ? <p className="mt-1 text-[11.5px] leading-snug text-zinc-400">{hint}</p> : null}
      </div>
    </div>
  );
}

// A small "live" pill — float it over a hero image to keep the realtime hook
// visible (e.g. "Tables update live"). Pure decoration; no client JS.
export function LiveBadge({ children }: { children: React.ReactNode }) {
  return (
    <span className="inline-flex items-center gap-2 rounded-full border border-zinc-200 bg-white/95 px-3 py-1.5 text-[12.5px] font-medium text-zinc-700 shadow-sm backdrop-blur">
      <span className="relative flex size-2">
        <span className="absolute inline-flex size-2 animate-ping rounded-full bg-green-500/60" />
        <span className="relative inline-flex size-2 rounded-full bg-green-600" />
      </span>
      {children}
    </span>
  );
}
