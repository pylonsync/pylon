import React from "react";
import { cn } from "@/lib/utils";

export interface ColumnDef<Row> {
  key: string;
  header: string;
  /** Cell content. Return a string and it's rendered as-is. */
  cell: (row: Row) => React.ReactNode;
  /** Right-align and tabular-figure a numeric column. */
  numeric?: boolean;
  className?: string;
}

/**
 * A dense list view. Rows are 36px so a screenful is a screenful of data, and
 * the header sticks while the body scrolls.
 *
 * Generic and presentational — every view here uses the same one, which is why
 * companies and contacts look like the same product.
 */
export function DataTable<Row extends { id: string }>({
  rows,
  columns,
  onRowClick,
  empty,
}: {
  rows: Row[];
  columns: ColumnDef<Row>[];
  onRowClick?: (row: Row) => void;
  empty?: React.ReactNode;
}) {
  if (rows.length === 0 && empty) return <>{empty}</>;

  return (
    <div className="min-h-0 flex-1 overflow-auto">
      <table className="w-full border-collapse text-[13px]">
        <thead className="sticky top-0 z-10 bg-background">
          <tr className="hairline">
            {columns.map((column) => (
              <th
                key={column.key}
                scope="col"
                className={cn(
                  "h-8 px-4 text-left text-[11px] font-medium text-muted-foreground",
                  column.numeric && "text-right",
                  column.className,
                )}
              >
                {column.header}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr
              key={row.id}
              onClick={onRowClick ? () => onRowClick(row) : undefined}
              tabIndex={onRowClick ? 0 : undefined}
              onKeyDown={
                onRowClick
                  ? (event) => {
                      if (event.key === "Enter") onRowClick(row);
                    }
                  : undefined
              }
              className={cn(
                "border-b border-border/60 transition-colors",
                onRowClick && "cursor-pointer hover:bg-surface-1",
              )}
            >
              {columns.map((column) => (
                <td
                  key={column.key}
                  className={cn(
                    "h-9 max-w-0 truncate px-4",
                    column.numeric && "tabular text-right",
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
