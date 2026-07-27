"use client";

import React, { useEffect, useState } from "react";
import { callFn, db, useRouter } from "@pylonsync/react";
import { Boxes, Plus, ArrowLeftRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/kbd";
import { PageHeader } from "@/components/page-header";
import { DataTable, type ColumnDef } from "@/components/data-table";
import { EmptyState } from "@/components/empty-state";
import { RecordDialog } from "@/components/record-dialog";
import { SummaryBar } from "@/components/summary-bar";
import { StockLevel } from "@/components/stock-level";
import { MovementDialog } from "@/components/movement-dialog";
import { money, parseAmount, parseCount, stockState } from "@/lib/stock";
import { Workspace, type ProductRow } from "./workspace";

export function ProductsView({
  email,
  openNew,
  initialFilter,
}: {
  email: string;
  openNew?: boolean;
  initialFilter?: string;
}) {
  const router = useRouter();
  const [newOpen, setNewOpen] = useState(Boolean(openNew));
  const [moveOpen, setMoveOpen] = useState(false);
  const [reorderOnly, setReorderOnly] = useState(initialFilter === "reorder");

  // "m" records a movement — the thing you do twenty times a day. Ignored
  // while typing.
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      const typing =
        !!target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.isContentEditable);
      if (typing || event.metaKey || event.ctrlKey) return;
      if (event.key === "m") {
        event.preventDefault();
        setMoveOpen(true);
      } else if (event.key === "c") {
        event.preventDefault();
        setNewOpen(true);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <Workspace email={email} pathname="/">
      {(data) => {
        const level = (id: string) => data.levels.get(id) ?? 0;

        const columns: ColumnDef<ProductRow>[] = [
          {
            key: "sku",
            header: "SKU",
            cell: (row) => <span className="tabular text-muted-foreground">{row.sku}</span>,
          },
          {
            key: "name",
            header: "Product",
            cell: (row) => <span className="truncate font-medium">{row.name}</span>,
          },
          {
            key: "category",
            header: "Category",
            cell: (row) => (
              <span className="text-muted-foreground">{row.category ?? "—"}</span>
            ),
          },
          {
            key: "onhand",
            header: "On hand",
            numeric: true,
            cell: (row) => (
              <StockLevel quantity={level(row.id)} reorderPoint={row.reorderPoint} />
            ),
          },
          {
            key: "reorder",
            header: "Reorder at",
            numeric: true,
            cell: (row) => (
              <span className="text-muted-foreground">{row.reorderPoint || "—"}</span>
            ),
          },
          {
            key: "value",
            header: "Value",
            numeric: true,
            cell: (row) =>
              money(Math.max(0, level(row.id)) * (Number(row.unitCostCents) || 0)),
          },
        ];

        const rows = data.products
          .filter((product) => !product.archived)
          .filter((product) =>
            reorderOnly
              ? stockState(level(product.id), product.reorderPoint) !== "ok"
              : true,
          )
          .sort((a, b) => a.sku.localeCompare(b.sku));

        return (
          <>
            <PageHeader title="Products" count={rows.length}>
              <Button
                size="sm"
                variant={reorderOnly ? "default" : "secondary"}
                onClick={() => setReorderOnly((on) => !on)}
              >
                Needs reorder
              </Button>
              <Button size="sm" variant="secondary" onClick={() => setMoveOpen(true)}>
                <ArrowLeftRight />
                Movement
                <Kbd className="ml-1">m</Kbd>
              </Button>
              <Button size="sm" onClick={() => setNewOpen(true)}>
                <Plus />
                New product
              </Button>
            </PageHeader>

            <SummaryBar products={data.products} movements={data.movements} />

            <DataTable
              rows={rows}
              columns={columns}
              onRowClick={(row) => router.push(`/products/${row.id}`)}
              empty={
                <EmptyState
                  icon={<Boxes />}
                  title={
                    data.loading
                      ? "Loading…"
                      : reorderOnly
                        ? "Nothing needs reordering"
                        : "No products yet"
                  }
                  description={
                    reorderOnly
                      ? "Every line is above its reorder point."
                      : "Add a product, then record what you receive. On-hand is the sum of those movements — there is no quantity to keep in sync."
                  }
                  action={
                    reorderOnly ? undefined : (
                      <Button size="sm" onClick={() => setNewOpen(true)}>
                        <Plus />
                        New product
                      </Button>
                    )
                  }
                />
              }
            />

            <RecordDialog
              open={newOpen}
              title="New product"
              submitLabel="Create product"
              onOpenChange={setNewOpen}
              fields={[
                { name: "sku", label: "SKU", required: true, placeholder: "CFE-001" },
                { name: "name", label: "Name", required: true, placeholder: "House Blend, 1kg" },
                { name: "category", label: "Category", placeholder: "Coffee" },
                { name: "unitCost", label: "Unit cost", placeholder: "9.40" },
                { name: "unitPrice", label: "Unit price", placeholder: "18.00" },
                { name: "reorderPoint", label: "Reorder at", placeholder: "12" },
              ]}
              onCreate={async (values) => {
                // Money and counts are parsed here, at the edge — everything
                // below this line is integers.
                await db.insert("Product", {
                  sku: values.sku,
                  name: values.name,
                  category: values.category ?? null,
                  unitCostCents: parseAmount(values.unitCost ?? "") ?? 0,
                  unitPriceCents: parseAmount(values.unitPrice ?? "") ?? 0,
                  reorderPoint: parseCount(values.reorderPoint ?? "") ?? 0,
                  archived: false,
                });
              }}
            />

            <MovementDialog
              open={moveOpen}
              products={data.products.filter((p) => !p.archived)}
              currentLevel={level}
              onOpenChange={setMoveOpen}
              onRecord={(productId, delta, reason, note) =>
                callFn("recordMovement", {
                  productId,
                  delta,
                  reason,
                  ...(note ? { note } : {}),
                })
              }
            />
          </>
        );
      }}
    </Workspace>
  );
}
