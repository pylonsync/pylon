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
import { FilterTabs } from "@/components/filter-tabs";
import { CategoryTile } from "@/components/category-tile";
import {
  STOCK_TABS,
  inStockTab,
  money,
  parseAmount,
  parseCount,
  stockState,
  stockTabCounts,
} from "@/lib/stock";
import { RequireAuth } from "@/components/require-auth";
import { Workspace, type ProductRow } from "./workspace";

export function ProductsView({
  openNew,
  initialFilter,
}: {
  openNew?: boolean;
  initialFilter?: string;
}) {
  const router = useRouter();
  const [newOpen, setNewOpen] = useState(Boolean(openNew));
  const [moveOpen, setMoveOpen] = useState(false);
  const [tab, setTab] = useState(initialFilter === "reorder" ? "reorder" : "all");

  // ⌘K "Needs reorder" navigates here with ?filter=reorder while the list may
  // already be mounted.
  useEffect(() => {
    if (initialFilter === "reorder") setTab("reorder");
  }, [initialFilter]);

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
    <RequireAuth>
      <Workspace pathname="/">
      {(data) => {
        const level = (id: string) => data.levels.get(id) ?? 0;

        const columns: ColumnDef<ProductRow>[] = [
          {
            key: "name",
            header: "Product",
            className: "w-[60%] md:w-[34%]",
            cell: (row) => (
              <span className="flex min-w-0 items-center gap-3">
                <CategoryTile category={row.category} />
                <span className="min-w-0">
                  <span className="block truncate font-medium">{row.name}</span>
                  <span className="block truncate font-mono text-[11.5px] text-muted-foreground md:hidden">
                    {row.sku}
                  </span>
                </span>
              </span>
            ),
          },
          {
            key: "sku",
            header: "SKU",
            hideBelow: "md",
            className: "w-[110px]",
            cell: (row) => <span className="font-mono text-[12px] text-muted-foreground">{row.sku}</span>,
          },
          {
            key: "category",
            header: "Category",
            hideBelow: "lg",
            cell: (row) => (
              <span className="text-muted-foreground">{row.category ?? "—"}</span>
            ),
          },
          {
            key: "onhand",
            header: "On hand",
            numeric: true,
            className: "w-[120px] md:w-[170px]",
            cell: (row) => (
              <StockLevel quantity={level(row.id)} reorderPoint={row.reorderPoint} />
            ),
          },
          {
            key: "reorder",
            header: "Reorder at",
            numeric: true,
            hideBelow: "lg",
            className: "w-[100px]",
            cell: (row) => (
              <span className="text-muted-foreground">{row.reorderPoint || "—"}</span>
            ),
          },
          {
            key: "value",
            header: "Value",
            numeric: true,
            hideBelow: "sm",
            className: "w-[110px]",
            cell: (row) =>
              money(Math.max(0, level(row.id)) * (Number(row.unitCostCents) || 0)),
          },
        ];

        const tabCounts = stockTabCounts(data.summary);
        const rows = data.products
          .filter((product) => !product.archived)
          .filter((product) =>
            inStockTab(tab, stockState(level(product.id), product.reorderPoint)),
          )
          .sort((a, b) => a.sku.localeCompare(b.sku));

        return (
          <>
            <PageHeader title="Products" count={data.navCounts["/"]}>
              <Button
                size="sm"
                variant="outline"
                onClick={() => setMoveOpen(true)}
                title="Record a movement (m)"
              >
                <ArrowLeftRight />
                <span className="max-sm:sr-only">Movement</span>
                <Kbd className="ml-0.5 hidden md:inline-flex">M</Kbd>
              </Button>
              <Button size="sm" onClick={() => setNewOpen(true)} title="New product (c)">
                <Plus />
                <span className="max-sm:sr-only">New product</span>
              </Button>
            </PageHeader>

            <SummaryBar summary={data.summary} />

            <FilterTabs
              label="Stock level"
              value={tab}
              onChange={setTab}
              tabs={STOCK_TABS.map((t) => ({ id: t.id, label: t.label, count: tabCounts[t.id] }))}
            />

            <DataTable
              rows={rows}
              columns={columns}
              loading={data.loading}
              label="Products"
              onRowClick={(row) => router.push(`/products/${row.id}`)}
              empty={
                <EmptyState
                  icon={<Boxes />}
                  title={
                    tab === "all"
                      ? "No products yet"
                      : tab === "out"
                        ? "Nothing is out of stock"
                        : "Nothing needs reordering"
                  }
                  description={
                    tab === "all"
                      ? "Add a product, then record what you receive. On-hand is the sum of those movements, so there is no quantity to keep in sync."
                      : "Every active product is above its reorder point."
                  }
                  action={
                    tab === "all" ? (
                      <Button size="sm" onClick={() => setNewOpen(true)}>
                        <Plus />
                        New product
                      </Button>
                    ) : undefined
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
    </RequireAuth>
  );
}
