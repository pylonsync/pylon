import React from "react";

// Reusable presentational pieces for the landing page. All server-rendered —
// no client JS. Restyle here and the whole page follows. The brand accent
// (`text-brand`, `bg-brand-soft`) comes from CSS vars set on <html> in
// app/layout.tsx, which read lib/site.config.ts — so re-theming is one edit.

// Shared container: a contained, centered column.
export const WRAP = "mx-auto w-full max-w-3xl px-6";

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
