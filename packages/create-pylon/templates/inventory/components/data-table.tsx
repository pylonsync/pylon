import React from "react";
import { cn } from "@/lib/utils";

export interface ColumnDef<Row> {
  key: string;
  header: string;
  /** Cell content. Return a string and it's rendered as-is. */
  cell: (row: Row) => React.ReactNode;
  /** Right-align and tabular-figure a numeric column. */
  numeric?: boolean;
  /**
   * Hide the column below a breakpoint. Phones keep the first column and the
   * one or two numbers that matter; the rest appear as the screen widens.
   */
  hideBelow?: "sm" | "md" | "lg" | "xl";
  /** Width and other classes, applied to the header and every cell. */
  className?: string;
}

const HIDE: Record<NonNullable<ColumnDef<unknown>["hideBelow"]>, string> = {
  sm: "hidden sm:table-cell",
  md: "hidden md:table-cell",
  lg: "hidden lg:table-cell",
  xl: "hidden xl:table-cell",
};

/**
 * A dense list view. Rows are 40px on desktop and 48px on touch screens, the
 * header sticks while the body scrolls, and the table uses a fixed layout so
 * column widths come from the column definitions, not the content.
 *
 * Keyboard: every clickable row is a Tab stop; ↑/↓ (or j/k) move between rows
 * and Enter opens one.
 */
export function DataTable<Row extends { id: string }>({
  rows,
  columns,
  onRowClick,
  loading,
  empty,
  label,
}: {
  rows: Row[];
  columns: ColumnDef<Row>[];
  onRowClick?: (row: Row) => void;
  /** Show placeholder rows while the first sync is in flight. */
  loading?: boolean;
  empty?: React.ReactNode;
  /** Accessible name for the table. */
  label?: string;
}) {
  if (!loading && rows.length === 0 && empty) return <>{empty}</>;

  function onRowKey(event: React.KeyboardEvent<HTMLTableRowElement>, row: Row) {
    const key = event.key;
    if (key === "Enter" && onRowClick) {
      event.preventDefault();
      onRowClick(row);
      return;
    }
    const down = key === "ArrowDown" || key === "j";
    const up = key === "ArrowUp" || key === "k";
    if (!down && !up) return;
    event.preventDefault();
    const target = (
      down ? event.currentTarget.nextElementSibling : event.currentTarget.previousElementSibling
    ) as HTMLElement | null;
    target?.focus();
  }

  return (
    <div className="min-h-0 flex-1 overflow-auto">
      <table aria-label={label} className="w-full table-fixed border-collapse text-[13px]">
        <thead className="sticky top-0 z-10 bg-background/95 backdrop-blur-sm">
          <tr className="hairline">
            {columns.map((column, index) => (
              <th
                key={column.key}
                scope="col"
                className={cn(
                  "h-9 px-3 text-left text-[11px] font-medium uppercase tracking-[0.04em] text-muted-foreground",
                  index === 0 && "pl-4 md:pl-5",
                  index === columns.length - 1 && "pr-4 md:pr-5",
                  column.numeric && "text-right",
                  column.hideBelow && HIDE[column.hideBelow],
                  column.className,
                )}
              >
                {column.header}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {loading
            ? Array.from({ length: 8 }).map((_, index) => (
                <tr key={`skeleton-${index}`} aria-hidden="true" className="border-b border-border/60">
                  {columns.map((column, c) => (
                    <td
                      key={column.key}
                      className={cn(
                        "h-12 px-3 md:h-10",
                        c === 0 && "pl-4 md:pl-5",
                        column.hideBelow && HIDE[column.hideBelow],
                        column.className,
                      )}
                    >
                      <div
                        className={cn(
                          "h-2.5 animate-pulse rounded-full bg-foreground/[0.07]",
                          column.numeric ? "ml-auto w-10" : c === 0 ? "w-3/4" : "w-1/2",
                        )}
                      />
                    </td>
                  ))}
                </tr>
              ))
            : rows.map((row) => (
                <tr
                  key={row.id}
                  onClick={onRowClick ? () => onRowClick(row) : undefined}
                  tabIndex={onRowClick ? 0 : undefined}
                  onKeyDown={onRowClick ? (event) => onRowKey(event, row) : undefined}
                  className={cn(
                    "border-b border-border/60 transition-colors",
                    onRowClick &&
                      "cursor-pointer hover:bg-foreground/[0.025] focus-visible:bg-primary/[0.06] focus-visible:outline-none",
                  )}
                >
                  {columns.map((column, index) => (
                    <td
                      key={column.key}
                      className={cn(
                        "h-12 truncate px-3 md:h-10",
                        index === 0 && "pl-4 md:pl-5",
                        index === columns.length - 1 && "pr-4 md:pr-5",
                        column.numeric && "tabular text-right",
                        column.hideBelow && HIDE[column.hideBelow],
                        column.className,
                      )}
                    >
                      {column.cell(row)}
                    </td>
                  ))}
                </tr>
              ))}
        </tbody>
      </table>
    </div>
  );
}
