"use client";

import React from "react";
import { Link, callFn, db, useRouter } from "@pylonsync/react";
import { ChevronRight, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Select } from "@/components/ui/select";
import { PageHeader } from "@/components/page-header";
import { EmptyState } from "@/components/empty-state";
import { ActivityTimeline } from "@/components/activity-timeline";
import { StageBadge } from "@/components/stage-badge";
import { Avatar } from "@/components/avatar";
import { PIPELINE, closeDue, money, relativeTime, shortDate } from "@/lib/pipeline";
import { RequireAuth } from "@/components/require-auth";
import { Workspace } from "../../workspace";

export function DealView({ dealId }: { dealId: string }) {
  const router = useRouter();

  return (
    <RequireAuth>
      <Workspace pathname="/">
      {(data) => {
        const deal = data.deals.find((d) => d.id === dealId);

        if (!deal) {
          return (
            <>
              <PageHeader title="Deal" />
              <EmptyState
                title={data.loading ? "Loading deal…" : "Deal not found"}
                description={
                  data.loading
                    ? undefined
                    : "It may have been deleted, or the link is wrong."
                }
                action={
                  data.loading ? undefined : (
                    <Button size="sm" variant="secondary" asChild>
                      <Link href="/">Back to pipeline</Link>
                    </Button>
                  )
                }
              />
            </>
          );
        }

        const contact = data.contacts.find((c) => c.id === deal.contactId);
        const activities = data.activities.filter((a) => a.dealId === deal.id);
        const due = closeDue(deal);
        const owner = data.ownerName(deal.ownerId);
        const company = data.companyName(deal.companyId);

        return (
          <>
            <PageHeader
              title={deal.title}
              leading={
                <Link
                  href="/"
                  className="hidden items-center gap-1 text-[13px] text-muted-foreground transition-colors hover:text-foreground md:inline-flex"
                >
                  Pipeline
                  <ChevronRight className="size-3.5" />
                </Link>
              }
            >
              <Button
                size="sm"
                variant="ghost"
                onClick={async () => {
                  await db.delete("Deal", deal.id);
                  router.push("/");
                }}
                className="text-muted-foreground hover:text-destructive"
              >
                <Trash2 />
                <span className="max-sm:sr-only">Delete</span>
              </Button>
            </PageHeader>

            <div className="min-h-0 flex-1 overflow-y-auto">
              <div className="mx-auto grid max-w-5xl gap-6 px-4 py-5 md:px-8 md:py-8 lg:grid-cols-[minmax(0,1fr)_280px] lg:gap-10">
                <div className="min-w-0 space-y-6">
                  <div>
                    <div className="mb-2 flex items-center gap-2 text-[12.5px]">
                      <StageBadge stage={deal.stage} />
                      {company ? (
                        <>
                          <span aria-hidden="true" className="text-muted-foreground/60">·</span>
                          <span className="truncate text-muted-foreground">{company}</span>
                        </>
                      ) : null}
                    </div>
                    <h2 className="text-[22px] font-semibold leading-tight tracking-[-0.02em] md:text-[26px]">
                      {deal.title}
                    </h2>
                    <p className="tabular mt-1.5 text-[20px] font-medium tracking-[-0.01em] text-muted-foreground">
                      {money(deal.value)}
                    </p>
                  </div>

                  <section aria-labelledby="activity-heading">
                    <h3 id="activity-heading" className="mb-3 text-[13px] font-semibold">
                      Activity
                      <span className="tabular ml-1.5 font-normal text-muted-foreground">
                        {activities.length}
                      </span>
                    </h3>
                    <ActivityTimeline
                      activities={activities}
                      ownerName={data.ownerName}
                      onLog={(kind, body) =>
                        callFn("logActivity", {
                          kind,
                          body,
                          dealId: deal.id,
                          ...(deal.companyId ? { companyId: deal.companyId } : {}),
                        })
                      }
                    />
                  </section>
                </div>

                <aside className="order-first lg:order-none">
                  <dl className="grid grid-cols-2 gap-x-4 gap-y-4 rounded-xl border border-border bg-surface-1 p-4 lg:grid-cols-1">
                    <Field label="Stage">
                      {/* The same mutation the board uses, so a change here
                          is logged the same way. */}
                      <Select
                        aria-label="Stage"
                        value={deal.stage}
                        className="h-8 bg-background md:h-7.5"
                        onChange={(event) =>
                          void callFn("moveDeal", {
                            dealId: deal.id,
                            stage: event.target.value,
                          })
                        }
                      >
                        {PIPELINE.map((stage) => (
                          <option key={stage.id} value={stage.id}>
                            {stage.label}
                          </option>
                        ))}
                      </Select>
                    </Field>

                    <Field label="Owner">
                      {owner ? (
                        <span className="flex min-w-0 items-center gap-2">
                          <Avatar name={owner} size="sm" />
                          <span className="truncate">{owner}</span>
                        </span>
                      ) : (
                        "—"
                      )}
                    </Field>

                    <Field label="Company">{company ?? "—"}</Field>

                    <Field label="Contact">
                      {contact ? (
                        <span className="flex min-w-0 items-center gap-2">
                          <Avatar name={contact.name} size="sm" />
                          <span className="min-w-0">
                            <span className="block truncate">{contact.name}</span>
                            {contact.email ? (
                              <a
                                href={`mailto:${contact.email}`}
                                className="block truncate text-[12px] text-muted-foreground hover:text-foreground"
                              >
                                {contact.email}
                              </a>
                            ) : null}
                          </span>
                        </span>
                      ) : (
                        "—"
                      )}
                    </Field>

                    <Field label="Expected close">
                      {deal.closeDate ? (
                        <span className="flex items-center gap-2">
                          <span className="tabular">{shortDate(deal.closeDate)}</span>
                          {due ? (
                            <span
                              className={
                                due.tone === "overdue"
                                  ? "text-[12px] font-medium text-destructive"
                                  : "text-[12px] text-muted-foreground"
                              }
                            >
                              {due.label}
                            </span>
                          ) : null}
                        </span>
                      ) : (
                        "—"
                      )}
                    </Field>

                    <Field label="Created">{relativeTime(deal.createdAt)}</Field>
                  </dl>
                </aside>
              </div>
            </div>
          </>
        );
      }}
      </Workspace>
    </RequireAuth>
  );
}

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="min-w-0">
      <dt className="text-[11.5px] font-medium text-muted-foreground">{label}</dt>
      <dd className="mt-1 min-w-0 truncate text-[13px]">{children}</dd>
    </div>
  );
}
