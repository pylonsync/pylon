"use client";

import React, { useState } from "react";
import { db } from "@pylonsync/react";
import { Plus, Users } from "lucide-react";
import { Button } from "@/components/ui/button";
import { PageHeader } from "@/components/page-header";
import { DataTable, type ColumnDef } from "@/components/data-table";
import { EmptyState } from "@/components/empty-state";
import { RecordDialog } from "@/components/record-dialog";
import { Avatar } from "@/components/avatar";
import { relativeTime } from "@/lib/pipeline";
import { RequireAuth } from "@/components/require-auth";
import { Workspace, type ContactRow } from "../workspace";

export function ContactsView() {
  const [open, setOpen] = useState(false);

  return (
    <RequireAuth>
      <Workspace pathname="/contacts">
      {(data) => {
        const columns: ColumnDef<ContactRow>[] = [
          {
            key: "name",
            header: "Name",
            className: "w-[55%] md:w-[22%]",
            cell: (row) => (
              <span className="flex min-w-0 items-center gap-2.5">
                <Avatar name={row.name} />
                <span className="min-w-0">
                  <span className="block truncate font-medium">{row.name}</span>
                  <span className="block truncate text-[12px] text-muted-foreground md:hidden">
                    {[row.title, data.companyName(row.companyId)].filter(Boolean).join(", ")}
                  </span>
                </span>
              </span>
            ),
          },
          {
            key: "title",
            header: "Title",
            hideBelow: "md",
            cell: (row) => (
              <span className="text-muted-foreground">{row.title ?? "—"}</span>
            ),
          },
          {
            key: "company",
            header: "Company",
            hideBelow: "md",
            cell: (row) => (
              <span className="text-muted-foreground">
                {data.companyName(row.companyId) ?? "—"}
              </span>
            ),
          },
          {
            key: "email",
            header: "Email",
            hideBelow: "lg",
            className: "w-[24%]",
            cell: (row) =>
              row.email ? (
                <a
                  href={`mailto:${row.email}`}
                  onClick={(event) => event.stopPropagation()}
                  className="text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
                >
                  {row.email}
                </a>
              ) : (
                <span className="text-muted-foreground">—</span>
              ),
          },
          {
            key: "phone",
            header: "Phone",
            className: "w-[140px]",
            cell: (row) =>
              row.phone ? (
                <a
                  href={`tel:${row.phone.replace(/[^+\d]/g, "")}`}
                  onClick={(event) => event.stopPropagation()}
                  className="tabular text-muted-foreground hover:text-foreground"
                >
                  {row.phone}
                </a>
              ) : (
                <span className="text-muted-foreground">—</span>
              ),
          },
          {
            key: "created",
            header: "Added",
            hideBelow: "xl",
            className: "w-[100px]",
            cell: (row) => (
              <span className="text-muted-foreground">
                {relativeTime(row.createdAt)}
              </span>
            ),
          },
        ];

        const rows = [...data.contacts].sort((a, b) => a.name.localeCompare(b.name));

        return (
          <>
            <PageHeader title="Contacts" count={data.navCounts["/contacts"]}>
              <Button size="sm" onClick={() => setOpen(true)}>
                <Plus />
                <span className="max-sm:sr-only">New contact</span>
              </Button>
            </PageHeader>

            <DataTable
              rows={rows}
              columns={columns}
              loading={data.loading}
              label="Contacts"
              empty={
                <EmptyState
                  icon={<Users />}
                  title="No contacts yet"
                  description="The people you actually talk to. Add one and link it to a company to keep the account together."
                  action={
                    <Button size="sm" onClick={() => setOpen(true)}>
                      <Plus />
                      New contact
                    </Button>
                  }
                />
              }
            />

            <RecordDialog
              open={open}
              title="New contact"
              submitLabel="Create contact"
              onOpenChange={setOpen}
              fields={[
                { name: "name", label: "Name", required: true, placeholder: "Dana Whitfield" },
                { name: "title", label: "Title", placeholder: "VP Operations" },
                {
                  name: "companyId",
                  label: "Company",
                  options: data.companies
                    .slice()
                    .sort((a, b) => a.name.localeCompare(b.name))
                    .map((company) => ({ value: company.id, label: company.name })),
                },
                { name: "email", label: "Email", type: "email", placeholder: "dana@northwind.co" },
                { name: "phone", label: "Phone", type: "tel", placeholder: "+1 555 0100" },
              ]}
              onCreate={async (values) => {
                await db.insert("Contact", values);
              }}
            />
          </>
        );
      }}
      </Workspace>
    </RequireAuth>
  );
}
