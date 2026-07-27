"use client";

import React, { useState } from "react";
import { Link, callFn, useRouter } from "@pylonsync/react";
import { ArrowLeft, ArrowLeftRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { PageHeader } from "@/components/page-header";
import { EmptyState } from "@/components/empty-state";
import { StockLevel } from "@/components/stock-level";
import { MovementDialog } from "@/components/movement-dialog";
import { relativeTime } from "@/lib/format";
import { money, reasonById, signedQuantity } from "@/lib/stock";
import { Workspace } from "../../workspace";

export function ProductView({
  email,
  productId,
}: {
  email: string;
  productId: string;
}) {
  const [moveOpen, setMoveOpen] = useState(false);

  return (
    <Workspace email={email} pathname="/">
      {(data) => {
        const product = data.products.find((p) => p.id === productId);

        if (!product) {
          return (
            <>
              <PageHeader title="Product" />
              <EmptyState
                title={data.loading ? "Loading…" : "Product not found"}
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
            <PageHeader title={product.name}>
              <Button size="sm" onClick={() => setMoveOpen(true)}>
                <ArrowLeftRight />
                Record movement
              </Button>
            </PageHeader>

            <div className="min-h-0 flex-1 overflow-y-auto">
              <div className="mx-auto flex max-w-3xl flex-col gap-6 px-6 py-6">
                <Link
                  href="/"
                  className="inline-flex w-fit items-center gap-1.5 text-[12px] text-muted-foreground transition-colors hover:text-foreground"
                >
                  <ArrowLeft className="size-3.5" />
                  Products
                </Link>

                <dl className="grid grid-cols-2 gap-x-6 gap-y-4 rounded-lg border border-border bg-card p-4 sm:grid-cols-4">
                  <Field label="SKU">
                    <span className="tabular">{product.sku}</span>
                  </Field>
                  <Field label="On hand">
                    <StockLevel
                      quantity={quantity}
                      reorderPoint={product.reorderPoint}
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
                    On hand is the sum of these movements. Nothing here is ever
                    edited — a correction is another movement.
                  </p>
                  {withBalance.length === 0 ? (
                    <p className="py-6 text-center text-[12px] text-muted-foreground">
                      No movements yet.
                    </p>
                  ) : (
                    <ol className="space-y-1.5">
                      {withBalance.map(({ movement, after }) => (
                        <li
                          key={movement.id}
                          className="flex items-center gap-3 rounded-md border border-border bg-card px-3 py-2 text-[12px]"
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
                            <span className="truncate text-muted-foreground">
                              {movement.note}
                            </span>
                          ) : null}
                          <span className="tabular ml-auto shrink-0 text-muted-foreground">
                            → {after}
                          </span>
                          <time
                            className="w-20 shrink-0 text-right text-muted-foreground"
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
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="min-w-0">
      <dt className="text-[11px] text-muted-foreground">{label}</dt>
      <dd className="mt-1 truncate text-[13px]">{children}</dd>
    </div>
  );
}
