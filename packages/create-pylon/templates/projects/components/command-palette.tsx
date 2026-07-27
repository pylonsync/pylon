import React, { useEffect, useMemo, useRef, useState } from "react";
import { Building2, Plus, Search, User } from "lucide-react";
import { cn } from "@/lib/utils";
import { Kbd } from "@/components/kbd";
import { moveSelection, searchItems, type SearchItem } from "@/lib/search";

const TYPE_ICON: Record<SearchItem["type"], React.ReactNode> = {
  deal: <Search />,
  company: <Building2 />,
  contact: <User />,
};

export interface CommandAction {
  id: string;
  label: string;
  run: () => void;
}

/**
 * ⌘K. Searches the synced replica in memory, so results land as you type with
 * no request per keystroke — see lib/search.ts for the ranking.
 *
 * Presentational: items and actions come in as props, selection is delegated
 * through onSelect. Keyboard handling lives here because it belongs to the
 * widget, not to the page that opened it.
 */
export function CommandPalette({
  open,
  items,
  actions = [],
  onClose,
  onSelect,
}: {
  open: boolean;
  items: SearchItem[];
  actions?: CommandAction[];
  onClose: () => void;
  onSelect: (item: SearchItem) => void;
}) {
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);

  const results = useMemo(() => searchItems(items, query), [items, query]);
  const shownActions = query.trim() ? [] : actions;
  const rows = useMemo(
    () => [
      ...shownActions.map((a) => ({ kind: "action" as const, action: a })),
      ...results.map((item) => ({ kind: "item" as const, item })),
    ],
    [shownActions, results],
  );

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setIndex(0);
    // Focus after paint so the dialog is mounted and the caret lands.
    const id = requestAnimationFrame(() => inputRef.current?.focus());
    return () => cancelAnimationFrame(id);
  }, [open]);

  useEffect(() => {
    setIndex(0);
  }, [query]);

  if (!open) return null;

  function commit(at: number) {
    const row = rows[at];
    if (!row) return;
    if (row.kind === "action") row.action.run();
    else onSelect(row.item);
    onClose();
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/50 pt-[12vh]"
      // Clicking the backdrop dismisses; clicks inside the panel must not.
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Search"
        className="w-full max-w-lg overflow-hidden rounded-xl border border-border bg-popover shadow-2xl"
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            onClose();
          } else if (event.key === "ArrowDown") {
            event.preventDefault();
            setIndex((i) => moveSelection(i, 1, rows.length));
          } else if (event.key === "ArrowUp") {
            event.preventDefault();
            setIndex((i) => moveSelection(i, -1, rows.length));
          } else if (event.key === "Enter") {
            event.preventDefault();
            commit(index);
          }
        }}
      >
        <div className="flex items-center gap-2 border-b border-border px-3">
          <Search className="size-4 shrink-0 text-muted-foreground" />
          <input
            ref={inputRef}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search deals, companies, contacts…"
            aria-label="Search deals, companies and contacts"
            className="h-11 flex-1 bg-transparent text-[13px] outline-none placeholder:text-muted-foreground"
          />
          <Kbd>esc</Kbd>
        </div>

        <div className="max-h-80 overflow-y-auto p-1.5">
          {rows.length === 0 ? (
            <p className="px-3 py-6 text-center text-[12px] text-muted-foreground">
              No matches for “{query}”
            </p>
          ) : (
            rows.map((row, at) => (
              <button
                key={row.kind === "action" ? row.action.id : row.item.id}
                type="button"
                // Pointer hover moves selection so mouse and keyboard agree.
                onMouseMove={() => setIndex(at)}
                onClick={() => commit(at)}
                aria-selected={at === index}
                className={cn(
                  "flex w-full items-center gap-2.5 rounded-md px-2.5 py-2 text-left text-[13px] transition-colors",
                  "[&_svg]:size-3.5 [&_svg]:shrink-0 [&_svg]:text-muted-foreground",
                  at === index ? "bg-surface-2" : "hover:bg-surface-2/60",
                )}
              >
                {row.kind === "action" ? (
                  <>
                    <Plus />
                    <span className="flex-1 truncate">{row.action.label}</span>
                  </>
                ) : (
                  <>
                    {TYPE_ICON[row.item.type]}
                    <span className="flex-1 truncate">{row.item.title}</span>
                    {row.item.subtitle ? (
                      <span className="truncate text-[12px] text-muted-foreground">
                        {row.item.subtitle}
                      </span>
                    ) : null}
                  </>
                )}
              </button>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
