import React from "react";
import type { Fact, MockRow } from "@/lib/site.config";

// Presentational pieces for the landing page. All server-rendered, no client
// JS. Colors come from CSS vars set on <html> in app/layout.tsx, which read
// lib/site.config.ts.

// Shared container: the page is left-aligned inside a wide column.
export const WRAP = "mx-auto w-full max-w-6xl px-6";

// Three short facts in a row, a thin rule above each.
export function FactList({ items }: { items: Fact[] }) {
  return (
    <ul className="grid gap-x-8 gap-y-10 sm:grid-cols-3">
      {items.map((f) => (
        <li key={f.title} className="border-t border-line pt-5">
          <h3 className="font-display text-[17px] font-medium text-chalk">{f.title}</h3>
          <p className="mt-2 text-[14px] leading-relaxed text-chalk-2">{f.body}</p>
        </li>
      ))}
    </ul>
  );
}

// A product window drawn with divs: a title bar, a sidebar, and task rows.
// No screenshot. Content comes from `siteConfig.mock`.
export function ProductMock({
  windowTitle,
  sidebar,
  activeSidebarIndex,
  rows,
}: {
  windowTitle: string;
  sidebar: string[];
  activeSidebarIndex: number;
  rows: MockRow[];
}) {
  return (
    <div
      aria-hidden
      className="overflow-hidden rounded-lg border border-line bg-paper text-[12px] text-chalk-2"
    >
      <div className="flex h-9 items-center gap-2 border-b border-line px-3">
        <span className="size-2.5 rounded-full bg-line" />
        <span className="size-2.5 rounded-full bg-line" />
        <span className="size-2.5 rounded-full bg-line" />
        <span className="ml-3 font-display text-[12px] font-medium text-chalk">{windowTitle}</span>
      </div>
      <div className="grid grid-cols-[112px_1fr] sm:grid-cols-[140px_1fr]">
        <div className="border-r border-line py-2">
          {sidebar.map((item, i) => (
            <div
              key={item}
              className={
                i === activeSidebarIndex
                  ? "mx-2 rounded-md bg-ink-2 px-2 py-1.5 text-chalk"
                  : "mx-2 px-2 py-1.5"
              }
            >
              {item}
            </div>
          ))}
        </div>
        <div className="divide-y divide-line">
          {rows.map((r) => (
            <div key={r.title} className="flex items-center gap-3 px-3 py-2.5">
              <span
                className={
                  r.done
                    ? "size-3.5 shrink-0 rounded-sm bg-brand"
                    : "size-3.5 shrink-0 rounded-sm border border-chalk-2/60"
                }
              />
              <span className={r.done ? "text-chalk-2 line-through" : "text-chalk"}>{r.title}</span>
              <span className="ml-auto font-mono-ui text-[11px] text-chalk-2">{r.tag}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
