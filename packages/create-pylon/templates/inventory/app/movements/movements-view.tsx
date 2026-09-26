"use client";

import React from "react";
import { ArrowLeftRight } from "lucide-react";
import { PageHeader } from "@/components/page-header";
import { DataTable, type ColumnDef } from "@/components/data-table";
import { EmptyState } from "@/components/empty-state";
import { Avatar } from "@/components/avatar";
import { relativeTime } from "@/lib/format";
import { reasonById, signedQuantity } from "@/lib/stock";
import { RequireAuth } from "@/components/require-auth";
import { Workspace, type MovementRow } from "../workspace";

/**
 * The ledger, newest first. Read-only by design: a movement is never edited or
 * deleted — the policy forbids it — because rewriting history would change a
 * past valuation and make the current level unexplainable. Corrections are new
 * rows in the opposite direction.
 */
export function MovementsView() {
  return (
    <RequireAuth>
      <Workspace pathname="/movements">
      {(data) => {
        const columns: ColumnDef<MovementRow>[] = [
          {
            key: "when",
            header: "When",
            className: "w-[84px] md:w-[100px]",
            cell: (row) => (
              <span className="text-muted-foreground">{relativeTime(row.createdAt)}</span>
            ),
          },
          {
            key: "product",
            header: "Product",
            className: "md:w-[30%]",
            cell: (row) => (
              <span className="min-w-0">
                <span className="block truncate font-medium">
                  {data.productName(row.productId) ?? "—"}
                </span>
                <span className="block truncate text-[12px] text-muted-foreground md:hidden">
                  {reasonById(row.reason)?.label ?? row.reason}
                </span>
              </span>
            ),
          },
          {
            key: "reason",
            header: "Reason",
            hideBelow: "md",
            className: "w-[130px]",
            cell: (row) => (
              <span className="text-muted-foreground">
                {reasonById(row.reason)?.label ?? row.reason}
              </span>
            ),
          },
          {
            key: "delta",
            header: "Change",
            numeric: true,
            className: "w-[80px] md:w-[90px]",
            cell: (row) => (
              <span
                className={
                  row.delta > 0 ? "text-stage-won" : row.delta < 0 ? "text-destructive" : ""
                }
              >
                {signedQuantity(row.delta)}
              </span>
            ),
          },
          {
            key: "note",
            header: "Note",
            hideBelow: "lg",
            cell: (row) => (
              <span className="text-muted-foreground">{row.note ?? "—"}</span>
            ),
          },
          {
            key: "who",
            header: "By",
            hideBelow: "sm",
            className: "w-[170px]",
            cell: (row) => {
              const name = data.actorName(row.actorId);
              return name ? (
                <span className="flex min-w-0 items-center gap-2">
                  <Avatar name={name} size="sm" />
                  <span className="truncate text-muted-foreground">{name}</span>
                </span>
              ) : (
                <span className="text-muted-foreground">—</span>
              );
            },
          },
        ];

        const rows = [...data.movements].sort((a, b) =>
          (b.createdAt ?? "").localeCompare(a.createdAt ?? ""),
        );

        return (
          <>
            <PageHeader title="Movements" count={data.navCounts["/movements"]} />
            <DataTable
              rows={rows}
              columns={columns}
              loading={data.loading}
              label="Stock movements"
              empty={
                <EmptyState
                  icon={<ArrowLeftRight />}
                  title="No movements yet"
                  description="Every change to stock is recorded here, permanently. On-hand is the sum of these rows."
                />
              }
            />
          </>
        );
      }}
      </Workspace>
    </RequireAuth>
  );
}
