import React from "react";

// Reusable presentational pieces for the marketing pages. All server-rendered,
// no client JS. The brand accent (`text-brand`) comes from CSS vars set on
// <html> in app/layout.tsx, which read lib/site.config.ts.

// Shared container: a contained, centered column.
export const WRAP = "mx-auto w-full max-w-5xl px-5 sm:px-8";

// Plain text link in the accent color. Used for nav, rail, and inline links.
export const LINK = "text-brand underline-offset-2 hover:underline";

export function Divider() {
  return (
    <div className={WRAP}>
      <div className="border-t border-zinc-200" />
    </div>
  );
}

export function SectionHead({ title, body }: { title: string; body?: string }) {
  return (
    <div>
      <h2 className="font-display text-[20px] leading-tight text-zinc-900">{title}</h2>
      {body ? <p className="mt-2 max-w-xl text-[14px] leading-relaxed text-zinc-600">{body}</p> : null}
    </div>
  );
}

// Initials for avatars, so a row looks finished without a photo.
export function initials(name: string) {
  return name
    .split(/\s+/)
    .map((w) => w[0])
    .join("")
    .slice(0, 2)
    .toUpperCase();
}

// An image slot. With `src` it renders the image; without, a dashed box with a
// title naming the image that belongs there and a hint on where to replace it.
//
//   shape  — "landscape" | "portrait" | "square" | "circle"
//   title  — what image belongs here
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
  /** A real image under public/images. */
  src?: string;
  className?: string;
}) {
  const aspect =
    shape === "portrait"
      ? "aspect-[4/5]"
      : shape === "square" || shape === "circle"
        ? "aspect-square"
        : "aspect-[4/3]";
  const radius = shape === "circle" ? "rounded-full" : "";
  if (src) {
    return (
      <div className={`relative overflow-hidden bg-zinc-100 ${aspect} ${radius} ${className}`}>
        <img src={src} alt={title} className="absolute inset-0 size-full object-cover" loading="lazy" />
      </div>
    );
  }
  return (
    <div
      className={`relative grid place-items-center overflow-hidden border border-dashed border-zinc-300 bg-zinc-50 ${aspect} ${radius} ${className}`}
    >
      <div className="px-4 text-center">
        <p className="font-mono text-[12px] text-zinc-500">{title}</p>
        {hint ? <p className="mt-1 text-[11px] leading-snug text-zinc-400">{hint}</p> : null}
      </div>
    </div>
  );
}
