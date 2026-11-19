import React from "react";

interface ShowcaseProps {
  /** The section id from `sections.ts`. */
  id: string;
  /** The component name. */
  title: string;
  /** The file that defines the component. */
  source: string;
  /** One sentence that says when to use the component. */
  description: string;
  children: React.ReactNode;
}

/** One entry on `/internal/components`: a name, a source file and live examples. */
export function Showcase({ id, title, source, description, children }: ShowcaseProps) {
  return (
    <section id={id} className="flex scroll-mt-10 flex-col gap-3">
      <header className="flex flex-col gap-1">
        <h2 className="text-lg font-semibold tracking-tight">{title}</h2>
        <code className="text-xs text-muted-foreground">{source}</code>
        <p className="text-sm text-muted-foreground">{description}</p>
      </header>
      <div className="flex flex-col gap-4 rounded-lg border p-4">{children}</div>
    </section>
  );
}

interface ExampleProps {
  /** The name of the variant or state. */
  label: string;
  children: React.ReactNode;
}

/** One labeled example in a `Showcase`. */
export function Example({ label, children }: ExampleProps) {
  return (
    <div className="flex flex-col gap-2">
      <span className="text-xs font-medium text-muted-foreground">{label}</span>
      <div className="flex flex-wrap items-center gap-3">{children}</div>
    </div>
  );
}
