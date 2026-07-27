"use client";

import React, { useState } from "react";
import { db } from "@pylonsync/react";
import { Building2, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { PageHeader } from "@/components/page-header";
import { DataTable, type ColumnDef } from "@/components/data-table";
import { EmptyState } from "@/components/empty-state";
import { RecordDialog } from "@/components/record-dialog";
import { Avatar } from "@/components/avatar";
import { money, relativeTime } from "@/lib/pipeline";
import { RequireAuth } from "@/components/require-auth";
import { Workspace, type CompanyRow, type DealRow } from "../workspace";

export function CompaniesView() {
  const [open, setOpen] = useState(false);

  return (
    <RequireAuth title="CRM" description="Your team shares one pipeline. Anyone with an account sees it.">
      <Workspace pathname="/companies">
      {(data) => {
        // Pipeline per company, computed here rather than stored — one less
        // thing to keep in sync when a deal moves.
        const openValue = (companyId: string) =>
          data.deals
            .filter((deal: DealRow) => deal.companyId === companyId)
            .reduce((sum, deal) => sum + (Number(deal.value) || 0), 0);

        const columns: ColumnDef<CompanyRow>[] = [
          {
            key: "name",
            header: "Company",
            cell: (row) => (
              <span className="flex items-center gap-2">
                <Avatar name={row.name} size="sm" />
                <span className="truncate font-medium">{row.name}</span>
              </span>
            ),
          },
          {
            key: "domain",
            header: "Domain",
            cell: (row) => (
              <span className="text-muted-foreground">{row.domain ?? "—"}</span>
            ),
          },
          {
            key: "industry",
            header: "Industry",
            cell: (row) => (
              <span className="text-muted-foreground">{row.industry ?? "—"}</span>
            ),
          },
          {
            key: "size",
            header: "Size",
            cell: (row) => (
              <span className="text-muted-foreground">{row.size ?? "—"}</span>
            ),
          },
          {
            key: "pipeline",
            header: "Pipeline",
            numeric: true,
            cell: (row) => money(openValue(row.id)),
          },
          {
            key: "created",
            header: "Added",
            cell: (row) => (
              <span className="text-muted-foreground">
                {relativeTime(row.createdAt)}
              </span>
            ),
          },
        ];

        const rows = [...data.companies].sort((a, b) => a.name.localeCompare(b.name));

        return (
          <>
            <PageHeader title="Companies" count={rows.length}>
              <Button size="sm" onClick={() => setOpen(true)}>
                <Plus />
                New company
              </Button>
            </PageHeader>

            <DataTable
              rows={rows}
              columns={columns}
              empty={
                <EmptyState
                  icon={<Building2 />}
                  title="No companies yet"
                  description="Companies are the accounts your deals and contacts hang off. Add the first one to start building a pipeline."
                  action={
                    <Button size="sm" onClick={() => setOpen(true)}>
                      <Plus />
                      New company
                    </Button>
                  }
                />
              }
            />

            <RecordDialog
              open={open}
              title="New company"
              submitLabel="Create company"
              onOpenChange={setOpen}
              fields={[
                { name: "name", label: "Name", required: true, placeholder: "Acme Inc" },
                { name: "domain", label: "Domain", placeholder: "acme.com" },
                { name: "industry", label: "Industry", placeholder: "Logistics" },
                { name: "size", label: "Size", placeholder: "50-200" },
              ]}
              onCreate={async (values) => {
                // A plain policy-checked insert — no server function needed,
                // because there's nothing to validate beyond the field types
                // and nothing to stamp that the policy doesn't already gate.
                await db.insert("Company", values);
              }}
            />
          </>
        );
      }}
      </Workspace>
    </RequireAuth>
  );
}
