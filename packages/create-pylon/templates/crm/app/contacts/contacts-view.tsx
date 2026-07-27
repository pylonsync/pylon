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
    <RequireAuth title="CRM" description="Your team shares one pipeline. Anyone with an account sees it.">
      <Workspace pathname="/contacts">
      {(data) => {
        const columns: ColumnDef<ContactRow>[] = [
          {
            key: "name",
            header: "Name",
            cell: (row) => (
              <span className="flex items-center gap-2">
                <Avatar name={row.name} size="sm" />
                <span className="truncate font-medium">{row.name}</span>
              </span>
            ),
          },
          {
            key: "title",
            header: "Title",
            cell: (row) => (
              <span className="text-muted-foreground">{row.title ?? "—"}</span>
            ),
          },
          {
            key: "company",
            header: "Company",
            cell: (row) => (
              <span className="text-muted-foreground">
                {data.companyName(row.companyId) ?? "—"}
              </span>
            ),
          },
          {
            key: "email",
            header: "Email",
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
            cell: (row) => (
              <span className="text-muted-foreground">{row.phone ?? "—"}</span>
            ),
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

        const rows = [...data.contacts].sort((a, b) => a.name.localeCompare(b.name));

        return (
          <>
            <PageHeader title="Contacts" count={rows.length}>
              <Button size="sm" onClick={() => setOpen(true)}>
                <Plus />
                New contact
              </Button>
            </PageHeader>

            <DataTable
              rows={rows}
              columns={columns}
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
                { name: "email", label: "Email", type: "email", placeholder: "dana@acme.com" },
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
