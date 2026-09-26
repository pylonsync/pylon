"use client";

import React, { useEffect, useState } from "react";
import { callFn, useRouter } from "@pylonsync/react";
import { Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/kbd";
import { PageHeader } from "@/components/page-header";
import { MetricsBar } from "@/components/metrics-bar";
import { PipelineBoard } from "@/components/pipeline-board";
import { BoardSkeleton } from "@/components/board-skeleton";
import { DealDialog } from "@/components/deal-dialog";
import { RequireAuth } from "@/components/require-auth";
import { Workspace } from "./workspace";

export function PipelineView({
  openNew,
}: {
  openNew?: boolean;
}) {
  const router = useRouter();
  const [dialogOpen, setDialogOpen] = useState(Boolean(openNew));

  // "c" creates a deal, the way an issue tracker does. Ignored while typing so
  // it never swallows a character in the search box or the activity composer.
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      const typing =
        !!target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.isContentEditable);
      if (event.key === "c" && !typing && !event.metaKey && !event.ctrlKey) {
        event.preventDefault();
        setDialogOpen(true);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <RequireAuth>
      <Workspace pathname="/">
      {(data) => (
        <>
          <PageHeader title="Pipeline" count={data.navCounts["/"]}>
            <Button size="sm" onClick={() => setDialogOpen(true)} title="New deal (c)">
              <Plus />
              New deal
              <Kbd className="ml-0.5 hidden border-primary-foreground/25 bg-primary-foreground/10 text-primary-foreground/75 md:inline-flex">
                C
              </Kbd>
            </Button>
          </PageHeader>

          <MetricsBar deals={data.deals} />

          <div className="min-h-0 flex-1">
            {data.loading ? (
              <BoardSkeleton />
            ) : (
              <PipelineBoard
                deals={data.deals}
                companyName={data.companyName}
                ownerName={data.ownerName}
                onOpen={(id) => router.push(`/deals/${id}`)}
                onMove={(dealId, stage) => {
                  // Through the mutation, not a bare update: moving a deal also
                  // writes a history entry, and the stage is validated
                  // server-side.
                  void callFn("moveDeal", { dealId, stage });
                }}
              />
            )}
          </div>

          <DealDialog
            open={dialogOpen}
            companies={data.companies}
            onOpenChange={setDialogOpen}
            onCreate={async (draft) => {
              await callFn("createDeal", {
                title: draft.title,
                value: draft.value,
                stage: draft.stage,
                ...(draft.companyId ? { companyId: draft.companyId } : {}),
                ...(draft.closeDate
                  ? { closeDate: new Date(draft.closeDate).toISOString() }
                  : {}),
              });
            }}
          />
        </>
      )}
      </Workspace>
    </RequireAuth>
  );
}
