"use client";

import React from "react";
import { ArrowLeftRight } from "lucide-react";
import { PageHeader } from "@/components/page-header";
import { DataTable, type ColumnDef } from "@/components/data-table";
import { EmptyState } from "@/components/empty-state";
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
    <RequireAuth title="Inventory" description="Your team shares one stock ledger. Anyone with an account sees it.">
      <Workspace pathname="/movements">
      {(data) => {
        const columns: ColumnDef<MovementRow>[] = [
          {
            key: "when",
            header: "When",
            cell: (row) => (
              <span className="text-muted-foreground">{relativeTime(row.createdAt)}</span>
            ),
          },
          {
            key: "product",
            header: "Product",
            cell: (row) => (
              <span className="truncate font-medium">
                {data.productName(row.productId) ?? "—"}
              </span>
            ),
          },
          {
            key: "reason",
            header: "Reason",
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
            cell: (row) => (
              <span className="text-muted-foreground">{row.note ?? "—"}</span>
            ),
          },
          {
            key: "who",
            header: "By",
            cell: (row) => (
              <span className="text-muted-foreground">
                {data.actorName(row.actorId) ?? "—"}
              </span>
            ),
          },
        ];

        const rows = [...data.movements].sort((a, b) =>
          (b.createdAt ?? "").localeCompare(a.createdAt ?? ""),
        );

        return (
          <>
            <PageHeader title="Movements" count={rows.length} />
            <DataTable
              rows={rows}
              columns={columns}
              empty={
                <EmptyState
                  icon={<ArrowLeftRight />}
                  title={data.loading ? "Loading…" : "No movements yet"}
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
