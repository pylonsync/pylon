import React from "react";
import { type Metadata } from "@pylonsync/react";

export const metadata: Metadata = {
  title: "Design tokens",
  robots: "noindex, nofollow",
};

// `/internal/design` shows every design token the app uses. The tokens are the
// CSS variables in `app/globals.css`. When you add or change a token there,
// add or change its entry here in the same commit.

// One entry for each color token in `app/globals.css`. Each class is written in
// full so that Tailwind finds it.
const COLORS = [
  { token: "background", swatch: "bg-background" },
  { token: "foreground", swatch: "bg-foreground" },
  { token: "card", swatch: "bg-card" },
  { token: "popover", swatch: "bg-popover" },
  { token: "primary", swatch: "bg-primary" },
  { token: "secondary", swatch: "bg-secondary" },
  { token: "muted", swatch: "bg-muted" },
  { token: "accent", swatch: "bg-accent" },
  { token: "destructive", swatch: "bg-destructive" },
  { token: "border", swatch: "bg-border" },
  { token: "input", swatch: "bg-input" },
  { token: "ring", swatch: "bg-ring" },
  { token: "chart-1", swatch: "bg-chart-1" },
  { token: "chart-2", swatch: "bg-chart-2" },
  { token: "chart-3", swatch: "bg-chart-3" },
  { token: "chart-4", swatch: "bg-chart-4" },
  { token: "chart-5", swatch: "bg-chart-5" },
  { token: "sidebar", swatch: "bg-sidebar" },
  { token: "sidebar-primary", swatch: "bg-sidebar-primary" },
  { token: "sidebar-accent", swatch: "bg-sidebar-accent" },
  { token: "sidebar-border", swatch: "bg-sidebar-border" },
  { token: "sidebar-ring", swatch: "bg-sidebar-ring" },
];

const TYPE_SCALE = [
  { name: "text-4xl", className: "text-4xl font-semibold tracking-tight" },
  { name: "text-3xl", className: "text-3xl font-semibold tracking-tight" },
  { name: "text-2xl", className: "text-2xl font-semibold tracking-tight" },
  { name: "text-xl", className: "text-xl font-semibold" },
  { name: "text-lg", className: "text-lg font-semibold" },
  { name: "text-base", className: "text-base" },
  { name: "text-sm", className: "text-sm" },
  { name: "text-xs", className: "text-xs" },
];

const RADII = [
  { name: "rounded-sm", className: "rounded-sm" },
  { name: "rounded-md", className: "rounded-md" },
  { name: "rounded-lg", className: "rounded-lg" },
  { name: "rounded-xl", className: "rounded-xl" },
  { name: "rounded-full", className: "rounded-full" },
];

const SPACING = [
  { name: "1", className: "w-1" },
  { name: "2", className: "w-2" },
  { name: "3", className: "w-3" },
  { name: "4", className: "w-4" },
  { name: "6", className: "w-6" },
  { name: "8", className: "w-8" },
  { name: "12", className: "w-12" },
  { name: "16", className: "w-16" },
];

const SHADOWS = [
  { name: "shadow-sm", className: "shadow-sm" },
  { name: "shadow-md", className: "shadow-md" },
  { name: "shadow-lg", className: "shadow-lg" },
];

function Section({
  id,
  title,
  children,
}: {
  /** The section id from `sections.ts`. */
  id: string;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section id={id} className="flex scroll-mt-10 flex-col gap-3">
      <h2 className="text-lg font-semibold tracking-tight">{title}</h2>
      {children}
    </section>
  );
}

export default function DesignPage() {
  return (
    <div className="flex flex-col gap-10">
      <header className="flex flex-col gap-2">
        <h1 className="text-3xl font-semibold tracking-tight">Design tokens</h1>
        <p className="text-sm text-muted-foreground">
          The tokens are CSS variables in <code>app/globals.css</code>. Use a
          token class such as <code>bg-primary</code>. Do not write a raw color,
          a raw radius or a raw shadow in a component.
        </p>
      </header>

      <Section id="color" title="Color">
        <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-4">
          {COLORS.map((color) => (
            <div key={color.token} className="flex flex-col gap-1.5">
              <div className={`h-14 rounded-lg border ${color.swatch}`} />
              <div className="flex flex-col text-xs">
                <span className="font-medium">{color.token}</span>
                <code className="text-muted-foreground">{color.swatch}</code>
              </div>
            </div>
          ))}
        </div>
      </Section>

      <Section id="type" title="Type">
        <div className="flex flex-col gap-3">
          {TYPE_SCALE.map((step) => (
            <div key={step.name} className="flex flex-col gap-0.5">
              <span className="text-xs text-muted-foreground">{step.name}</span>
              <span className={step.className}>The quick brown fox</span>
            </div>
          ))}
        </div>
      </Section>

      <Section id="radius" title="Radius">
        <div className="flex flex-wrap gap-4">
          {RADII.map((radius) => (
            <div key={radius.name} className="flex flex-col items-center gap-1.5">
              <div className={`size-14 border bg-muted ${radius.className}`} />
              <span className="text-xs text-muted-foreground">{radius.name}</span>
            </div>
          ))}
        </div>
      </Section>

      <Section id="spacing" title="Spacing">
        <div className="flex flex-col gap-2">
          {SPACING.map((space) => (
            <div key={space.name} className="flex items-center gap-3">
              <span className="w-6 text-xs text-muted-foreground">{space.name}</span>
              <div className={`h-3 rounded-sm bg-primary ${space.className}`} />
            </div>
          ))}
        </div>
      </Section>

      <Section id="shadow" title="Shadow">
        <div className="flex flex-wrap gap-6">
          {SHADOWS.map((shadow) => (
            <div key={shadow.name} className="flex flex-col items-center gap-1.5">
              <div className={`size-14 rounded-lg border bg-card ${shadow.className}`} />
              <span className="text-xs text-muted-foreground">{shadow.name}</span>
            </div>
          ))}
        </div>
      </Section>
    </div>
  );
}
