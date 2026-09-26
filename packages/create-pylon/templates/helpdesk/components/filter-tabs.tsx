import React from "react";
import { cn } from "@/lib/utils";

export interface FilterTab {
  id: string;
  label: string;
  count?: number;
}

/**
 * A row of view filters under the page header, each with its count. On a
 * phone the row scrolls sideways inside itself instead of widening the page.
 *
 * Each filter is a toggle button in a labelled group; ←/→ move between them
 * and Home/End jump to the ends.
 */
export function FilterTabs({
  tabs,
  value,
  onChange,
  label,
  trailing,
  className,
}: {
  tabs: FilterTab[];
  value: string;
  onChange: (id: string) => void;
  /** Accessible name for the group, such as "Ticket status". */
  label: string;
  /** Extra controls at the end of the row, such as a toggle. */
  trailing?: React.ReactNode;
  className?: string;
}) {
  function onKeyDown(event: React.KeyboardEvent<HTMLButtonElement>, index: number) {
    const target =
      event.key === "ArrowRight"
        ? (index + 1) % tabs.length
        : event.key === "ArrowLeft"
          ? (index - 1 + tabs.length) % tabs.length
          : event.key === "Home"
            ? 0
            : event.key === "End"
              ? tabs.length - 1
              : -1;
    if (target < 0) return;
    event.preventDefault();
    const next = tabs[target];
    onChange(next.id);
    const list = event.currentTarget.parentElement;
    list?.querySelector<HTMLButtonElement>(`[data-tab="${next.id}"]`)?.focus();
  }

  return (
    <div
      className={cn(
        "flex h-11 shrink-0 items-center gap-3 border-b border-border px-3 md:px-5",
        className,
      )}
    >
      <div
        role="group"
        aria-label={label}
        className="-mx-1 flex min-w-0 flex-1 items-center gap-0.5 overflow-x-auto px-1 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
      >
        {tabs.map((tab, index) => {
          const selected = tab.id === value;
          return (
            <button
              key={tab.id}
              type="button"
              data-tab={tab.id}
              aria-pressed={selected}
              tabIndex={selected ? 0 : -1}
              onClick={() => onChange(tab.id)}
              onKeyDown={(event) => onKeyDown(event, index)}
              className={cn(
                "flex h-7 shrink-0 items-center gap-1.5 rounded-md px-2.5 text-[12.5px] transition-colors",
                selected
                  ? "bg-foreground/[0.07] font-medium text-foreground"
                  : "text-muted-foreground hover:bg-foreground/[0.04] hover:text-foreground",
              )}
            >
              {tab.label}
              {typeof tab.count === "number" ? (
                <span
                  className={cn(
                    "tabular text-[11.5px]",
                    selected ? "text-foreground/60" : "text-muted-foreground/80",
                  )}
                >
                  {tab.count}
                </span>
              ) : null}
            </button>
          );
        })}
      </div>
      {trailing ? <div className="flex shrink-0 items-center gap-2">{trailing}</div> : null}
    </div>
  );
}
