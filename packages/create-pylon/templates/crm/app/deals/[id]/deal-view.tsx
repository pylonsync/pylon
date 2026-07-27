"use client";

import React from "react";
import { Link, callFn, db, useRouter } from "@pylonsync/react";
import { ArrowLeft, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Select } from "@/components/ui/select";
import { PageHeader } from "@/components/page-header";
import { EmptyState } from "@/components/empty-state";
import { ActivityTimeline } from "@/components/activity-timeline";
import { StageBadge } from "@/components/stage-badge";
import { Avatar } from "@/components/avatar";
import { PIPELINE, daysUntil, money, relativeTime } from "@/lib/pipeline";
import { RequireAuth } from "@/components/require-auth";
import { Workspace } from "../../workspace";

export function DealView({ dealId }: { dealId: string }) {
  const router = useRouter();

  return (
    <RequireAuth title="CRM" description="Your team shares one pipeline. Anyone with an account sees it.">
      <Workspace pathname="/">
      {(data) => {
        const deal = data.deals.find((d) => d.id === dealId);

        if (!deal) {
          return (
            <>
              <PageHeader title="Deal" />
              <EmptyState
                title={data.loading ? "Loading…" : "Deal not found"}
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
        const days = daysUntil(deal.closeDate);

        return (
          <>
            <PageHeader title={deal.title}>
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
                Delete
              </Button>
            </PageHeader>

            <div className="min-h-0 flex-1 overflow-y-auto">
              <div className="mx-auto flex max-w-3xl flex-col gap-6 px-6 py-6">
                <Link
                  href="/"
                  className="inline-flex w-fit items-center gap-1.5 text-[12px] text-muted-foreground transition-colors hover:text-foreground"
                >
                  <ArrowLeft className="size-3.5" />
                  Pipeline
                </Link>

                <div className="flex flex-wrap items-baseline gap-x-4 gap-y-2">
                  <h1 className="text-[22px] font-semibold tracking-tight">
                    {deal.title}
                  </h1>
                  <span className="tabular text-[18px] font-medium text-muted-foreground">
                    {money(deal.value)}
                  </span>
                </div>

                <dl className="grid grid-cols-2 gap-x-6 gap-y-4 rounded-lg border border-border bg-card p-4 sm:grid-cols-4">
                  <Field label="Stage">
                    {/* Changing stage here goes through the same mutation the
                        board uses, so it's logged identically. */}
                    <Select
                      aria-label="Stage"
                      value={deal.stage}
                      className="h-7 text-[12px]"
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

                  <Field label="Company">
                    {data.companyName(deal.companyId) ?? "—"}
                  </Field>

                  <Field label="Contact">
                    {contact ? (
                      <span className="flex items-center gap-1.5">
                        <Avatar name={contact.name} size="sm" />
                        {contact.name}
                      </span>
                    ) : (
                      "—"
                    )}
                  </Field>

                  <Field label="Expected close">
                    {deal.closeDate ? (
                      <span className={days !== null && days < 0 ? "text-destructive" : ""}>
                        {new Date(deal.closeDate).toLocaleDateString("en-US", {
                          month: "short",
                          day: "numeric",
                          year: "numeric",
                        })}
                      </span>
                    ) : (
                      "—"
                    )}
                  </Field>
                </dl>

                <div className="flex items-center gap-3 text-[12px] text-muted-foreground">
                  <StageBadge stage={deal.stage} />
                  <span aria-hidden="true">·</span>
                  <span>Created {relativeTime(deal.createdAt)}</span>
                  {data.ownerName(deal.ownerId) ? (
                    <>
                      <span aria-hidden="true">·</span>
                      <span>Owned by {data.ownerName(deal.ownerId)}</span>
                    </>
                  ) : null}
                </div>

                <section>
                  <h2 className="mb-3 text-[13px] font-semibold">Activity</h2>
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
      <dt className="text-[11px] text-muted-foreground">{label}</dt>
      <dd className="mt-1 truncate text-[13px]">{children}</dd>
    </div>
  );
}
