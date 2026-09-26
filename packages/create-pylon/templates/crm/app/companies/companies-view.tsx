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
import { isOpen, money, relativeTime } from "@/lib/pipeline";
import { RequireAuth } from "@/components/require-auth";
import { Workspace, type CompanyRow, type DealRow } from "../workspace";

export function CompaniesView() {
  const [open, setOpen] = useState(false);

  return (
    <RequireAuth>
      <Workspace pathname="/companies">
      {(data) => {
        // Open pipeline per company, computed from the deals rather than
        // stored, so a moved deal can't leave it stale.
        const openValue = (companyId: string) =>
          data.deals
            .filter((deal: DealRow) => deal.companyId === companyId && isOpen(deal))
            .reduce((sum, deal) => sum + (Number(deal.value) || 0), 0);

        const columns: ColumnDef<CompanyRow>[] = [
          {
            key: "name",
            header: "Company",
            className: "w-[46%] md:w-[26%]",
            cell: (row) => (
              <span className="flex min-w-0 items-center gap-2.5">
                <Avatar name={row.name} shape="square" />
                <span className="min-w-0">
                  <span className="block truncate font-medium">{row.name}</span>
                  <span className="block truncate text-[12px] text-muted-foreground md:hidden">
                    {row.industry ?? row.domain ?? ""}
                  </span>
                </span>
              </span>
            ),
          },
          {
            key: "domain",
            header: "Domain",
            hideBelow: "md",
            cell: (row) => (
              <span className="text-muted-foreground">{row.domain ?? "—"}</span>
            ),
          },
          {
            key: "industry",
            header: "Industry",
            hideBelow: "md",
            cell: (row) => (
              <span className="text-muted-foreground">{row.industry ?? "—"}</span>
            ),
          },
          {
            key: "size",
            header: "Size",
            hideBelow: "lg",
            className: "w-[110px]",
            cell: (row) => (
              <span className="text-muted-foreground">{row.size ?? "—"}</span>
            ),
          },
          {
            key: "pipeline",
            header: "Open pipeline",
            numeric: true,
            className: "w-[130px]",
            cell: (row) => {
              const value = openValue(row.id);
              return value > 0 ? money(value) : <span className="text-muted-foreground">—</span>;
            },
          },
          {
            key: "created",
            header: "Added",
            hideBelow: "sm",
            className: "w-[110px]",
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
            <PageHeader title="Companies" count={data.navCounts["/companies"]}>
              <Button size="sm" onClick={() => setOpen(true)}>
                <Plus />
                <span className="max-sm:sr-only">New company</span>
              </Button>
            </PageHeader>

            <DataTable
              rows={rows}
              columns={columns}
              loading={data.loading}
              label="Companies"
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
                { name: "name", label: "Name", required: true, placeholder: "Northwind Logistics" },
                { name: "domain", label: "Domain", placeholder: "northwind.co" },
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
