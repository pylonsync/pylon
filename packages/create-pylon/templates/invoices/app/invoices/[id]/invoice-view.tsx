"use client";

import React, { useState } from "react";
import { Link, callFn, db, useRouter } from "@pylonsync/react";
import { ArrowLeft, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Select } from "@/components/ui/select";
import { PageHeader } from "@/components/page-header";
import { EmptyState } from "@/components/empty-state";
import { StatusBadge } from "@/components/status-badge";
import { LineItems } from "@/components/line-items";
import { TotalsPanel } from "@/components/totals-panel";
import { PaymentDialog } from "@/components/payment-dialog";
import { relativeTime } from "@/lib/format";
import { STATUSES, displayStatus, money, totals } from "@/lib/billing";
import { RequireAuth } from "@/components/require-auth";
import { Workspace } from "../../workspace";

export function InvoiceView({
  invoiceId,
}: {
  invoiceId: string;
}) {
  const router = useRouter();
  const [payOpen, setPayOpen] = useState(false);

  return (
    <RequireAuth title="Invoices" description="Your team shares one set of books. Anyone with an account sees it.">
      <Workspace pathname="/">
      {(data) => {
        const invoice = data.invoices.find((i) => i.id === invoiceId);

        if (!invoice) {
          return (
            <>
              <PageHeader title="Invoice" />
              <EmptyState
                title={data.loading ? "Loading…" : "Invoice not found"}
                description={
                  data.loading ? undefined : "It may have been deleted, or the link is wrong."
                }
                action={
                  data.loading ? undefined : (
                    <Button size="sm" variant="secondary" asChild>
                      <Link href="/">Back to invoices</Link>
                    </Button>
                  )
                }
              />
            </>
          );
        }

        const t = totals(invoice, data.items, data.payments);
        const shown = displayStatus(invoice, t.balanceCents);
        const client = data.clients.find((c) => c.id === invoice.clientId);
        const lines = data.items.filter((item) => item.invoiceId === invoice.id);
        const paid = data.payments
          .filter((payment) => payment.invoiceId === invoice.id)
          .sort((a, b) => (b.paidAt ?? "").localeCompare(a.paidAt ?? ""));
        // A sent invoice is a document someone is paying against; editing its
        // lines after the fact desyncs it from the copy in their inbox.
        const editable = invoice.status === "draft";

        return (
          <>
            <PageHeader title={invoice.number}>
              <StatusBadge status={shown} />
              <div className="w-28">
                <Select
                  aria-label="Status"
                  value={invoice.status}
                  className="h-7 text-[12px]"
                  onChange={(event) =>
                    void callFn("setInvoiceStatus", {
                      invoiceId: invoice.id,
                      status: event.target.value,
                    })
                  }
                >
                  {STATUSES.map((s) => (
                    <option key={s.id} value={s.id}>
                      {s.label}
                    </option>
                  ))}
                </Select>
              </div>
              {t.balanceCents > 0 && invoice.status !== "void" ? (
                <Button size="sm" onClick={() => setPayOpen(true)}>
                  Record payment
                </Button>
              ) : null}
              {editable ? (
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={async () => {
                    await db.delete("Invoice", invoice.id);
                    router.push("/");
                  }}
                  className="text-muted-foreground hover:text-destructive"
                >
                  <Trash2 />
                </Button>
              ) : null}
            </PageHeader>

            <div className="min-h-0 flex-1 overflow-y-auto">
              <div className="mx-auto flex max-w-3xl flex-col gap-6 px-6 py-6">
                <Link
                  href="/"
                  className="inline-flex w-fit items-center gap-1.5 text-[12px] text-muted-foreground transition-colors hover:text-foreground"
                >
                  <ArrowLeft className="size-3.5" />
                  Invoices
                </Link>

                <dl className="grid grid-cols-2 gap-x-6 gap-y-4 rounded-lg border border-border bg-card p-4 sm:grid-cols-4">
                  <Field label="Client">
                    <Select
                      aria-label="Client"
                      value={invoice.clientId ?? ""}
                      className="h-7 text-[12px]"
                      onChange={(event) =>
                        void db.update("Invoice", invoice.id, {
                          clientId: event.target.value || null,
                        })
                      }
                    >
                      <option value="">—</option>
                      {data.clients.map((c) => (
                        <option key={c.id} value={c.id}>
                          {c.name}
                        </option>
                      ))}
                    </Select>
                  </Field>
                  <Field label="Issued">
                    {invoice.issueDate
                      ? new Date(invoice.issueDate).toLocaleDateString("en-US", {
                          month: "short",
                          day: "numeric",
                          year: "numeric",
                        })
                      : "—"}
                  </Field>
                  <Field label="Due">
                    {invoice.dueDate
                      ? new Date(invoice.dueDate).toLocaleDateString("en-US", {
                          month: "short",
                          day: "numeric",
                          year: "numeric",
                        })
                      : "—"}
                  </Field>
                  <Field label="Balance">
                    <span
                      className={
                        "tabular font-medium" +
                        (t.balanceCents > 0 ? " text-destructive" : "")
                      }
                    >
                      {money(t.balanceCents)}
                    </span>
                  </Field>
                </dl>

                {client?.address ? (
                  <div className="text-[12px] text-muted-foreground">
                    <p className="mb-1 font-medium text-foreground">Billed to</p>
                    <p className="whitespace-pre-wrap">{client.address}</p>
                  </div>
                ) : null}

                <LineItems
                  items={lines}
                  editable={editable}
                  onAdd={async (draft) => {
                    await db.insert("LineItem", {
                      ...draft,
                      invoiceId: invoice.id,
                      position: lines.length,
                    });
                  }}
                  onRemove={async (id) => {
                    await db.delete("LineItem", id);
                  }}
                />

                <TotalsPanel totals={t} taxRateBps={invoice.taxRateBps} />

                {paid.length > 0 ? (
                  <section>
                    <h2 className="mb-2 text-[13px] font-semibold">Payments</h2>
                    <ul className="space-y-1.5">
                      {paid.map((payment) => (
                        <li
                          key={payment.id}
                          className="flex items-center gap-3 rounded-md border border-border bg-card px-3 py-2 text-[12px]"
                        >
                          <span className="tabular font-medium">
                            {money(payment.amountCents)}
                          </span>
                          <span className="text-muted-foreground capitalize">
                            {payment.method ?? "other"}
                          </span>
                          {payment.reference ? (
                            <span className="truncate text-muted-foreground">
                              {payment.reference}
                            </span>
                          ) : null}
                          <time
                            className="ml-auto text-muted-foreground"
                            dateTime={payment.paidAt ?? undefined}
                          >
                            {relativeTime(payment.paidAt)}
                          </time>
                        </li>
                      ))}
                    </ul>
                  </section>
                ) : null}
              </div>
            </div>

            <PaymentDialog
              open={payOpen}
              balanceCents={t.balanceCents}
              onOpenChange={setPayOpen}
              onRecord={(amountCents, method, reference) =>
                callFn("recordPayment", {
                  invoiceId: invoice.id,
                  amountCents,
                  method,
                  ...(reference ? { reference } : {}),
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
      <dt className="text-[11px] text-muted-foreground">{label}</dt>
      <dd className="mt-1 truncate text-[13px]">{children}</dd>
    </div>
  );
}
