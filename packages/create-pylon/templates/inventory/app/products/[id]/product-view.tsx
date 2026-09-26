"use client";

import React, { useState } from "react";
import { Link, callFn, useRouter } from "@pylonsync/react";
import { ArrowLeftRight, ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { PageHeader } from "@/components/page-header";
import { EmptyState } from "@/components/empty-state";
import { StockLevel } from "@/components/stock-level";
import { CategoryTile } from "@/components/category-tile";
import { Avatar } from "@/components/avatar";
import { MovementDialog } from "@/components/movement-dialog";
import { relativeTime } from "@/lib/format";
import { money, reasonById, signedQuantity } from "@/lib/stock";
import { RequireAuth } from "@/components/require-auth";
import { Workspace } from "../../workspace";

export function ProductView({
  productId,
}: {
  productId: string;
}) {
  const [moveOpen, setMoveOpen] = useState(false);

  return (
    <RequireAuth>
      <Workspace pathname="/">
      {(data) => {
        const product = data.products.find((p) => p.id === productId);

        if (!product) {
          return (
            <>
              <PageHeader title="Product" />
              <EmptyState
                title={data.loading ? "Loading product…" : "Product not found"}
                description={
                  data.loading ? undefined : "It may have been deleted, or the link is wrong."
                }
                action={
                  data.loading ? undefined : (
                    <Button size="sm" variant="secondary" asChild>
                      <Link href="/">Back to products</Link>
                    </Button>
                  )
                }
              />
            </>
          );
        }

        const quantity = data.levels.get(product.id) ?? 0;
        // Newest first, and running backwards from the current level so each
        // row shows what the shelf held right after it — the question you
        // actually ask when auditing a count.
        const history = data.movements
          .filter((movement) => movement.productId === product.id)
          .sort((a, b) => (b.createdAt ?? "").localeCompare(a.createdAt ?? ""));
        let running = quantity;
        const withBalance = history.map((movement) => {
          const after = running;
          running -= Math.trunc(Number(movement.delta) || 0);
          return { movement, after };
        });

        return (
          <>
            <PageHeader
              title={product.name}
              leading={
                <Link
                  href="/"
                  className="hidden items-center gap-1 text-[13px] text-muted-foreground transition-colors hover:text-foreground md:inline-flex"
                >
                  Products
                  <ChevronRight className="size-3.5" />
                </Link>
              }
            >
              <Button size="sm" onClick={() => setMoveOpen(true)}>
                <ArrowLeftRight />
                <span className="max-sm:sr-only">Record movement</span>
              </Button>
            </PageHeader>

            <div className="min-h-0 flex-1 overflow-y-auto">
              <div className="mx-auto flex max-w-3xl flex-col gap-6 px-4 py-5 md:px-8 md:py-8">
                <div className="flex items-center gap-3.5">
                  <CategoryTile category={product.category} className="size-11 rounded-lg [&_svg]:size-5" />
                  <div className="min-w-0">
                    <h2 className="truncate text-[20px] font-semibold tracking-[-0.015em] md:text-[22px]">
                      {product.name}
                    </h2>
                    <p className="mt-0.5 text-[12.5px] text-muted-foreground">
                      <span className="font-mono">{product.sku}</span>
                      {product.category ? ` · ${product.category}` : ""}
                    </p>
                  </div>
                </div>

                <dl className="grid grid-cols-2 gap-x-6 gap-y-4 rounded-xl border border-border bg-surface-1 p-4 sm:grid-cols-4">
                  <Field label="Reorder at">
                    <span className="tabular">{product.reorderPoint || "—"}</span>
                  </Field>
                  <Field label="On hand">
                    <StockLevel
                      quantity={quantity}
                      reorderPoint={product.reorderPoint}
                      className="justify-start [&>span:last-child]:min-w-0 [&>span:last-child]:text-left"
                    />
                  </Field>
                  <Field label="Unit cost">{money(product.unitCostCents)}</Field>
                  <Field label="Stock value">
                    {money(Math.max(0, quantity) * (Number(product.unitCostCents) || 0))}
                  </Field>
                </dl>

                <section>
                  <h2 className="mb-2 text-[13px] font-semibold">History</h2>
                  <p className="mb-3 text-[12px] text-muted-foreground">
                    On hand is the sum of these movements. Movements are never
                    edited; a correction is another movement.
                  </p>
                  {withBalance.length === 0 ? (
                    <p className="py-6 text-center text-[12px] text-muted-foreground">
                      No movements yet.
                    </p>
                  ) : (
                    <ol className="divide-y divide-border/70 overflow-hidden rounded-xl border border-border">
                      {withBalance.map(({ movement, after }) => (
                        <li
                          key={movement.id}
                          className="flex h-11 items-center gap-3 bg-card px-3.5 text-[13px]"
                        >
                          <span
                            className={
                              "tabular w-12 shrink-0 font-medium " +
                              (movement.delta > 0 ? "text-stage-won" : "text-destructive")
                            }
                          >
                            {signedQuantity(movement.delta)}
                          </span>
                          <span className="shrink-0">
                            {reasonById(movement.reason)?.label ?? movement.reason}
                          </span>
                          {movement.note ? (
                            <span className="hidden truncate text-muted-foreground sm:inline">
                              {movement.note}
                            </span>
                          ) : null}
                          <span className="ml-auto hidden shrink-0 items-center gap-1.5 text-[12.5px] text-muted-foreground md:flex">
                            {data.actorName(movement.actorId) ? (
                              <>
                                <Avatar name={data.actorName(movement.actorId)} size="sm" />
                                {data.actorName(movement.actorId)}
                              </>
                            ) : null}
                          </span>
                          <span
                            className="tabular ml-auto w-14 shrink-0 text-right text-muted-foreground md:ml-0"
                            title="On hand after this movement"
                          >
                            → {after}
                          </span>
                          <time
                            className="w-16 shrink-0 text-right text-muted-foreground"
                            dateTime={movement.createdAt ?? undefined}
                          >
                            {relativeTime(movement.createdAt)}
                          </time>
                        </li>
                      ))}
                    </ol>
                  )}
                </section>
              </div>
            </div>

            <MovementDialog
              open={moveOpen}
              products={data.products.filter((p) => !p.archived)}
              currentLevel={(id) => data.levels.get(id) ?? 0}
              defaultProductId={product.id}
              onOpenChange={setMoveOpen}
              onRecord={(pid, delta, reason, note) =>
                callFn("recordMovement", {
                  productId: pid,
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

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="min-w-0">
      <dt className="text-[11.5px] font-medium text-muted-foreground">{label}</dt>
      <dd className="mt-1 truncate text-[14px] font-medium">{children}</dd>
    </div>
  );
}
