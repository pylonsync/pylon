"use client";

import React, { useEffect, useMemo, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { useAuth } from "@pylonsync/client";
import {
  money,
  parseLineItems,
  lineItemsTotal,
  type InquiryRow,
  type OwnerInquiriesResult,
  type ProjectRow,
  type ClientRow,
  type InvoiceRow,
  type InvoiceLineItem,
  type OwnerClientsResult,
  type OwnerInvoicesResult,
} from "@/lib/agency";
import { siteConfig } from "@/lib/site.config";

// The studio back-office. One owner-gated dashboard with four tabs:
//   • Pipeline — live inquiries + the public "slots open" capacity counter.
//   • Work     — the portfolio. Projects are public-read, so this reads them
//                with a LIVE `db.useQuery("Project")` (drafts included) and
//                toggling `selected` re-curates the homepage in real time.
//   • Clients  — the CRM. PII, so it loads through the owner-gated
//                `clientsForOwner` and never travels over entity sync.
//   • Invoices — billing. Money + client data, owner-gated like Clients.
//
// The whole dashboard is gated to PYLON_OWNER_EMAIL: one owner probe up front
// (inquiriesForOwner) decides access; a non-owner sees the owner-only card.
type Tab = "pipeline" | "work" | "clients" | "invoices";

export function AgencyDashboard({ userEmail }: { userEmail: string }) {
  const [tab, setTab] = useState<Tab>("pipeline");
  const [authorized, setAuthorized] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const r = await callFn<OwnerInquiriesResult>("inquiriesForOwner", {});
        if (cancelled) return;
        if (!r.authorized) {
          setAuthorized(false);
          return;
        }
        setAuthorized(true);
        // Seed demo clients + invoices once, for the owner only (idempotent).
        void callFn("seedStudioBackoffice", {}).catch(() => {});
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  if (error) {
    return <div className="rounded-xl border border-red-200 bg-red-50 px-5 py-4 text-sm text-red-700">{error}</div>;
  }
  if (authorized === null) return <Skeleton />;
  if (!authorized) return <OwnerOnly email={userEmail} />;

  const tabs: { id: Tab; label: string }[] = [
    { id: "pipeline", label: "Pipeline" },
    { id: "work", label: "Work" },
    { id: "clients", label: "Clients" },
    { id: "invoices", label: "Invoices" },
  ];

  return (
    <div className="space-y-6">
      <nav className="flex gap-1 border-b border-zinc-200">
        {tabs.map((t) => (
          <button
            key={t.id}
            type="button"
            onClick={() => setTab(t.id)}
            className={
              "-mb-px border-b-2 px-3.5 py-2.5 text-[13.5px] font-medium transition-colors " +
              (tab === t.id
                ? "border-brand text-zinc-900"
                : "border-transparent text-zinc-500 hover:text-zinc-800")
            }
          >
            {t.label}
          </button>
        ))}
      </nav>

      {tab === "pipeline" ? <PipelinePanel /> : null}
      {tab === "work" ? <WorkPanel /> : null}
      {tab === "clients" ? <ClientsPanel /> : null}
      {tab === "invoices" ? <InvoicesPanel /> : null}
    </div>
  );
}

/* =============================== PIPELINE =============================== */

interface CapacityRow {
  id: string;
  label: string;
  openSlots: number;
  updatedAt: string;
}

// Liveness rides the public Capacity row: `db.useQuery("Capacity")` re-renders
// the instant slots change (cross-tab). Leads come from the owner-gated
// inquiriesForOwner, refetched whenever capacity changes (which is exactly when
// a lead is booked/released), so the pipeline stays live without PII syncing.
function PipelinePanel() {
  const { data: caps } = db.useQuery<CapacityRow>("Capacity");
  const cap = caps[0];
  const liveKey = `${cap?.openSlots ?? "?"}:${cap?.label ?? ""}`;

  const [inquiries, setInquiries] = useState<InquiryRow[] | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  async function load() {
    const r = await callFn<OwnerInquiriesResult>("inquiriesForOwner", {});
    if (r.authorized) setInquiries(r.inquiries);
  }

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [liveKey]);

  async function act(id: string, fn: "bookInquiry" | "declineInquiry") {
    setBusyId(id);
    try {
      await callFn(fn, { inquiryId: id });
      await load();
    } finally {
      setBusyId(null);
    }
  }

  if (!inquiries) return <Skeleton compact />;

  const newCount = inquiries.filter((i) => i.status === "new").length;
  const bookedCount = inquiries.filter((i) => i.status === "booked").length;
  const active = inquiries.filter((i) => i.status !== "declined");

  return (
    <div className="space-y-6">
      <PanelHead title="Pipeline" sub="Live — leads land here the moment they're sent. Booking one drops the open-slot count on your site instantly." />

      <div className="grid gap-4 sm:grid-cols-3">
        <Stat label="Open slots" value={String(cap?.openSlots ?? 0)} hint={cap?.label} />
        <Stat label="New leads" value={String(newCount)} />
        <Stat label="Booked" value={String(bookedCount)} />
      </div>

      <CapacityCard cap={cap} />

      <Card title="Inquiries" count={active.length}>
        {inquiries.length === 0 ? (
          <Empty>No inquiries yet — share your site.</Empty>
        ) : (
          <ul className="divide-y divide-zinc-100">
            {inquiries.map((i) => (
              <li key={i.id} className="px-4 py-3.5">
                <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-2">
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-[14px] font-medium text-zinc-900">{i.name}</span>
                      <Pill kind="inquiry" status={i.status} />
                    </div>
                    <div className="mt-0.5 truncate text-[12.5px] text-zinc-500">
                      {i.email}
                      {i.company ? ` · ${i.company}` : ""}
                      {i.projectType ? ` · ${i.projectType}` : ""}
                      {i.budget ? ` · ${i.budget}` : ""}
                    </div>
                    {i.message ? (
                      <p className="mt-1.5 line-clamp-2 text-[13px] leading-relaxed text-zinc-600">{i.message}</p>
                    ) : null}
                  </div>
                  <div className="flex items-center gap-2">
                    {i.status !== "booked" ? (
                      <button
                        type="button"
                        disabled={busyId === i.id}
                        onClick={() => act(i.id, "bookInquiry")}
                        className="rounded-md bg-brand px-3 py-1.5 text-[12.5px] font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-50"
                      >
                        {busyId === i.id ? "…" : "Book"}
                      </button>
                    ) : null}
                    {i.status !== "declined" ? (
                      <button
                        type="button"
                        disabled={busyId === i.id}
                        onClick={() => act(i.id, "declineInquiry")}
                        className="rounded-md border border-zinc-300 px-3 py-1.5 text-[12.5px] font-medium text-zinc-600 transition-colors hover:border-red-300 hover:text-red-600 disabled:opacity-50"
                      >
                        {i.status === "booked" ? "Release" : "Decline"}
                      </button>
                    ) : null}
                  </div>
                </div>
              </li>
            ))}
          </ul>
        )}
      </Card>
    </div>
  );
}

function CapacityCard({ cap }: { cap?: CapacityRow }) {
  const [label, setLabel] = useState(cap?.label ?? "");
  const [slots, setSlots] = useState(String(cap?.openSlots ?? 0));
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    setLabel(cap?.label ?? "");
    setSlots(String(cap?.openSlots ?? 0));
  }, [cap?.label, cap?.openSlots]);

  async function save(e: React.FormEvent) {
    e.preventDefault();
    setSaving(true);
    setSaved(false);
    try {
      await callFn("setCapacity", { label: label.trim(), openSlots: Math.max(0, parseInt(slots, 10) || 0) });
      setSaved(true);
    } finally {
      setSaving(false);
    }
  }

  return (
    <form onSubmit={save} className="rounded-xl border border-zinc-200 bg-white p-4">
      <div className="text-sm font-semibold text-zinc-900">Availability</div>
      <p className="mt-1 text-[13px] text-zinc-500">Shown live on your site as “N project slots open”.</p>
      <div className="mt-4 flex flex-wrap items-end gap-3">
        <label className="block">
          <span className="mb-1 block text-[12px] font-medium text-zinc-600">Booking window</span>
          <input
            value={label}
            onChange={(e) => { setLabel(e.target.value); setSaved(false); }}
            placeholder="Q3 2026"
            className="h-9 w-40 rounded-lg border border-zinc-300 px-3 text-[14px] outline-none focus:border-brand focus:ring-2 focus:ring-brand/20"
          />
        </label>
        <label className="block">
          <span className="mb-1 block text-[12px] font-medium text-zinc-600">Open slots</span>
          <input
            type="number"
            min={0}
            value={slots}
            onChange={(e) => { setSlots(e.target.value); setSaved(false); }}
            className="h-9 w-24 rounded-lg border border-zinc-300 px-3 text-[14px] tabular-nums outline-none focus:border-brand focus:ring-2 focus:ring-brand/20"
          />
        </label>
        <button
          type="submit"
          disabled={saving}
          className="h-9 rounded-lg bg-zinc-900 px-4 text-[13px] font-medium text-white transition-colors hover:bg-zinc-700 disabled:opacity-50"
        >
          {saving ? "Saving…" : saved ? "Saved ✓" : "Save"}
        </button>
      </div>
    </form>
  );
}

/* ================================= WORK ================================ */

// Projects are public-read, so this is a LIVE query — toggling `selected` here
// updates the homepage's "Selected work" on its next render, and any other open
// dashboard tab instantly. Writes go through the owner-gated functions.
function WorkPanel() {
  const { data: projects } = db.useQuery<ProjectRow>("Project");
  const [editing, setEditing] = useState<ProjectRow | "new" | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  const ordered = useMemo(
    () => [...projects].sort((a, b) => a.order - b.order || (a.createdAt < b.createdAt ? -1 : 1)),
    [projects],
  );
  const selectedCount = projects.filter((p) => p.selected && p.published).length;

  async function flag(id: string, patch: { selected?: boolean; published?: boolean }) {
    setBusyId(id);
    try {
      await callFn("setProjectFlags", { id, ...patch });
    } finally {
      setBusyId(null);
    }
  }
  async function remove(p: ProjectRow) {
    if (!confirm(`Delete “${p.title}”? This can't be undone.`)) return;
    setBusyId(p.id);
    try {
      await callFn("deleteProject", { id: p.id });
    } finally {
      setBusyId(null);
    }
  }

  return (
    <div className="space-y-6">
      <PanelHead
        title="Work"
        sub={`Your portfolio. ${selectedCount} featured on the homepage. Toggle the star to feature, the eye to publish.`}
        action={<NewButton onClick={() => setEditing("new")}>New project</NewButton>}
      />

      <Card title="Projects" count={projects.length}>
        {ordered.length === 0 ? (
          <Empty>No projects yet — add your first case study.</Empty>
        ) : (
          <ul className="divide-y divide-zinc-100">
            {ordered.map((p) => (
              <li key={p.id} className="flex items-center gap-3 px-4 py-3">
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="truncate text-[14px] font-medium text-zinc-900">{p.title}</span>
                    {!p.published ? (
                      <span className="rounded bg-zinc-100 px-1.5 py-0.5 text-[10px] font-medium text-zinc-500">draft</span>
                    ) : null}
                    {p.selected && p.published ? (
                      <span className="rounded bg-amber-50 px-1.5 py-0.5 text-[10px] font-medium text-amber-700">featured</span>
                    ) : null}
                  </div>
                  <div className="mt-0.5 truncate text-[12.5px] text-zinc-500">
                    {p.client}
                    {p.summary ? ` · ${p.summary}` : ""}
                  </div>
                </div>
                <div className="flex shrink-0 items-center gap-1">
                  <IconToggle
                    on={p.selected}
                    disabled={busyId === p.id}
                    title={p.selected ? "Featured on homepage" : "Feature on homepage"}
                    onClick={() => flag(p.id, { selected: !p.selected })}
                  >
                    {p.selected ? "★" : "☆"}
                  </IconToggle>
                  <IconToggle
                    on={p.published}
                    disabled={busyId === p.id}
                    title={p.published ? "Published" : "Draft (hidden)"}
                    onClick={() => flag(p.id, { published: !p.published })}
                  >
                    {p.published ? "👁" : "🚫"}
                  </IconToggle>
                  <a
                    href={`/work/${p.slug}`}
                    target="_blank"
                    rel="noreferrer"
                    className="rounded-md px-2 py-1 text-[12px] text-zinc-500 hover:bg-zinc-100 hover:text-zinc-800"
                    title="View case study"
                  >
                    ↗
                  </a>
                  <button
                    type="button"
                    onClick={() => setEditing(p)}
                    className="rounded-md px-2.5 py-1 text-[12.5px] font-medium text-zinc-600 hover:bg-zinc-100"
                  >
                    Edit
                  </button>
                  <button
                    type="button"
                    disabled={busyId === p.id}
                    onClick={() => remove(p)}
                    className="rounded-md px-2 py-1 text-[12.5px] text-zinc-400 hover:bg-red-50 hover:text-red-600 disabled:opacity-50"
                    title="Delete"
                  >
                    ✕
                  </button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </Card>

      {editing ? (
        <ProjectForm
          initial={editing === "new" ? null : editing}
          nextOrder={projects.length}
          onClose={() => setEditing(null)}
        />
      ) : null}
    </div>
  );
}

function ProjectForm({
  initial,
  nextOrder,
  onClose,
}: {
  initial: ProjectRow | null;
  nextOrder: number;
  onClose: () => void;
}) {
  const [f, setF] = useState({
    title: initial?.title ?? "",
    client: initial?.client ?? "",
    summary: initial?.summary ?? "",
    year: initial?.year ?? "",
    tags: initial?.tags ?? "",
    selected: initial?.selected ?? false,
    published: initial?.published ?? true,
    order: initial?.order ?? nextOrder,
    challenge: initial?.challenge ?? "",
    approach: initial?.approach ?? "",
    outcome: initial?.outcome ?? "",
    liveUrl: initial?.liveUrl ?? "",
  });
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const set = <K extends keyof typeof f>(k: K, v: (typeof f)[K]) => setF((s) => ({ ...s, [k]: v }) as typeof f);

  async function save() {
    if (!f.title.trim()) { setErr("A title is required."); return; }
    setSaving(true);
    setErr(null);
    try {
      await callFn("upsertProject", {
        id: initial?.id,
        title: f.title.trim(),
        client: f.client.trim() || undefined,
        summary: f.summary.trim() || undefined,
        year: f.year.trim() || undefined,
        tags: f.tags.trim() || undefined,
        selected: f.selected,
        published: f.published,
        order: Number(f.order) || 0,
        challenge: f.challenge.trim() || undefined,
        approach: f.approach.trim() || undefined,
        outcome: f.outcome.trim() || undefined,
        liveUrl: f.liveUrl.trim() || undefined,
      });
      onClose();
    } catch (e) {
      setErr(e instanceof Error ? e.message : "Couldn't save — try again.");
      setSaving(false);
    }
  }

  return (
    <Modal title={initial ? "Edit project" : "New project"} onClose={onClose} onSubmit={save} saving={saving} error={err}>
      <Field label="Title" required>
        <input value={f.title} onChange={(e) => set("title", e.target.value)} className={inputCls} autoFocus />
      </Field>
      <div className="grid gap-3 sm:grid-cols-2">
        <Field label="Client label" hint="e.g. Fintech · 0→1">
          <input value={f.client} onChange={(e) => set("client", e.target.value)} className={inputCls} />
        </Field>
        <Field label="Year">
          <input value={f.year} onChange={(e) => set("year", e.target.value)} placeholder="2026" className={inputCls} />
        </Field>
      </div>
      <Field label="Summary" hint="One line, shown on the card">
        <input value={f.summary} onChange={(e) => set("summary", e.target.value)} className={inputCls} />
      </Field>
      <Field label="Tags" hint="Comma-separated">
        <input value={f.tags} onChange={(e) => set("tags", e.target.value)} placeholder="Product design, iOS" className={inputCls} />
      </Field>
      <Field label="The challenge">
        <textarea value={f.challenge} onChange={(e) => set("challenge", e.target.value)} rows={2} className={inputCls + " resize-none py-2"} />
      </Field>
      <Field label="Our approach">
        <textarea value={f.approach} onChange={(e) => set("approach", e.target.value)} rows={2} className={inputCls + " resize-none py-2"} />
      </Field>
      <Field label="The outcome">
        <textarea value={f.outcome} onChange={(e) => set("outcome", e.target.value)} rows={2} className={inputCls + " resize-none py-2"} />
      </Field>
      <div className="grid gap-3 sm:grid-cols-2">
        <Field label="Live URL">
          <input value={f.liveUrl} onChange={(e) => set("liveUrl", e.target.value)} placeholder="https://…" className={inputCls} />
        </Field>
        <Field label="Order" hint="Lower shows first">
          <input type="number" value={f.order} onChange={(e) => set("order", Number(e.target.value))} className={inputCls + " tabular-nums"} />
        </Field>
      </div>
      <div className="flex flex-wrap gap-5 pt-1">
        <Switch label="Feature on homepage" checked={f.selected} onChange={(v) => set("selected", v)} />
        <Switch label="Published" checked={f.published} onChange={(v) => set("published", v)} />
      </div>
    </Modal>
  );
}

/* =============================== CLIENTS =============================== */

function ClientsPanel() {
  const [clients, setClients] = useState<ClientRow[] | null>(null);
  const [editing, setEditing] = useState<ClientRow | "new" | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  async function load() {
    const r = await callFn<OwnerClientsResult>("clientsForOwner", {});
    if (r.authorized) setClients(r.clients);
  }
  useEffect(() => { void load(); }, []);

  async function remove(c: ClientRow) {
    if (!confirm(`Delete ${c.name}?`)) return;
    setBusyId(c.id);
    setNote(null);
    try {
      const r = await callFn<{ ok: boolean; invoices: number }>("deleteClient", { id: c.id });
      if (!r.ok) setNote(`${c.name} has ${r.invoices} invoice${r.invoices === 1 ? "" : "s"} — delete those first.`);
      else await load();
    } finally {
      setBusyId(null);
    }
  }

  if (!clients) return <Skeleton compact />;

  return (
    <div className="space-y-6">
      <PanelHead
        title="Clients"
        sub="Your CRM — private contact details, never exposed to the public site."
        action={<NewButton onClick={() => setEditing("new")}>New client</NewButton>}
      />
      {note ? (
        <div className="rounded-lg border border-amber-200 bg-amber-50 px-4 py-2.5 text-[13px] text-amber-800">{note}</div>
      ) : null}

      <Card title="Contacts" count={clients.length}>
        {clients.length === 0 ? (
          <Empty>No clients yet — add your first contact.</Empty>
        ) : (
          <ul className="divide-y divide-zinc-100">
            {clients.map((c) => (
              <li key={c.id} className="flex items-start gap-3 px-4 py-3.5">
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-[14px] font-medium text-zinc-900">{c.name}</span>
                    {c.company ? <span className="text-[13px] text-zinc-500">{c.company}</span> : null}
                    <Pill kind="client" status={c.status} />
                  </div>
                  <div className="mt-0.5 truncate text-[12.5px] text-zinc-500">
                    {[c.email, c.phone].filter(Boolean).join(" · ") || "No contact details"}
                  </div>
                  {c.notes ? <p className="mt-1 line-clamp-2 text-[13px] leading-relaxed text-zinc-600">{c.notes}</p> : null}
                </div>
                <div className="flex shrink-0 items-center gap-1">
                  <button type="button" onClick={() => setEditing(c)} className="rounded-md px-2.5 py-1 text-[12.5px] font-medium text-zinc-600 hover:bg-zinc-100">
                    Edit
                  </button>
                  <button
                    type="button"
                    disabled={busyId === c.id}
                    onClick={() => remove(c)}
                    className="rounded-md px-2 py-1 text-[12.5px] text-zinc-400 hover:bg-red-50 hover:text-red-600 disabled:opacity-50"
                    title="Delete"
                  >
                    ✕
                  </button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </Card>

      {editing ? (
        <ClientForm initial={editing === "new" ? null : editing} onClose={() => setEditing(null)} onSaved={load} />
      ) : null}
    </div>
  );
}

function ClientForm({
  initial,
  onClose,
  onSaved,
}: {
  initial: ClientRow | null;
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const [f, setF] = useState({
    name: initial?.name ?? "",
    company: initial?.company ?? "",
    email: initial?.email ?? "",
    phone: initial?.phone ?? "",
    status: initial?.status ?? "prospect",
    notes: initial?.notes ?? "",
  });
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const set = <K extends keyof typeof f>(k: K, v: (typeof f)[K]) => setF((s) => ({ ...s, [k]: v }) as typeof f);

  async function save() {
    if (!f.name.trim()) { setErr("A name is required."); return; }
    setSaving(true);
    setErr(null);
    try {
      await callFn("upsertClient", {
        id: initial?.id,
        name: f.name.trim(),
        company: f.company.trim() || undefined,
        email: f.email.trim() || undefined,
        phone: f.phone.trim() || undefined,
        status: f.status,
        notes: f.notes.trim() || undefined,
      });
      await onSaved();
      onClose();
    } catch (e) {
      setErr(e instanceof Error ? e.message : "Couldn't save — try again.");
      setSaving(false);
    }
  }

  return (
    <Modal title={initial ? "Edit client" : "New client"} onClose={onClose} onSubmit={save} saving={saving} error={err}>
      <div className="grid gap-3 sm:grid-cols-2">
        <Field label="Name" required>
          <input value={f.name} onChange={(e) => set("name", e.target.value)} className={inputCls} autoFocus />
        </Field>
        <Field label="Company">
          <input value={f.company} onChange={(e) => set("company", e.target.value)} className={inputCls} />
        </Field>
        <Field label="Email">
          <input type="email" value={f.email} onChange={(e) => set("email", e.target.value)} className={inputCls} />
        </Field>
        <Field label="Phone">
          <input value={f.phone} onChange={(e) => set("phone", e.target.value)} className={inputCls} />
        </Field>
      </div>
      <Field label="Status">
        <select value={f.status} onChange={(e) => set("status", e.target.value)} className={inputCls}>
          <option value="prospect">Prospect</option>
          <option value="active">Active</option>
          <option value="past">Past</option>
        </select>
      </Field>
      <Field label="Notes">
        <textarea value={f.notes} onChange={(e) => set("notes", e.target.value)} rows={3} className={inputCls + " resize-none py-2"} />
      </Field>
    </Modal>
  );
}

/* =============================== INVOICES ============================== */

function InvoicesPanel() {
  const [invoices, setInvoices] = useState<InvoiceRow[] | null>(null);
  const [clients, setClients] = useState<ClientRow[]>([]);
  const { data: projects } = db.useQuery<ProjectRow>("Project");
  const [editing, setEditing] = useState<InvoiceRow | "new" | null>(null);
  const [viewing, setViewing] = useState<InvoiceRow | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  async function load() {
    const [inv, cl] = await Promise.all([
      callFn<OwnerInvoicesResult>("invoicesForOwner", {}),
      callFn<OwnerClientsResult>("clientsForOwner", {}),
    ]);
    if (inv.authorized) setInvoices(inv.invoices);
    if (cl.authorized) setClients(cl.clients);
    // Keep an open view in sync after an edit.
    if (inv.authorized) {
      setViewing((cur) => (cur ? inv.invoices.find((i) => i.id === cur.id) ?? null : null));
    }
  }
  useEffect(() => { void load(); }, []);

  const clientFor = (inv: InvoiceRow) => clients.find((c) => c.id === inv.clientId);

  async function setStatus(id: string, status: string) {
    setBusyId(id);
    try {
      await callFn("setInvoiceStatus", { id, status });
      await load();
    } finally {
      setBusyId(null);
    }
  }
  async function remove(inv: InvoiceRow) {
    if (!confirm(`Delete ${inv.number}?`)) return;
    setBusyId(inv.id);
    try {
      await callFn("deleteInvoice", { id: inv.id });
      await load();
    } finally {
      setBusyId(null);
    }
  }

  const totals = useMemo(() => {
    const list = invoices ?? [];
    const paid = list.filter((i) => i.status === "paid").reduce((s, i) => s + i.amountCents, 0);
    const outstanding = list
      .filter((i) => i.status === "sent" || i.status === "overdue")
      .reduce((s, i) => s + i.amountCents, 0);
    return { paid, outstanding };
  }, [invoices]);

  if (!invoices) return <Skeleton compact />;

  return (
    <div className="space-y-6">
      <PanelHead
        title="Invoices"
        sub="Billing — private. View any invoice, export it to PDF, and track what's paid vs outstanding. Never syncs to the public site."
        action={<NewButton onClick={() => setEditing("new")}>New invoice</NewButton>}
      />

      <div className="grid gap-4 sm:grid-cols-3">
        <Stat label="Paid" value={money(totals.paid)} />
        <Stat label="Outstanding" value={money(totals.outstanding)} />
        <Stat label="Invoices" value={String(invoices.length)} />
      </div>

      <Card title="All invoices" count={invoices.length}>
        {invoices.length === 0 ? (
          <Empty>No invoices yet.</Empty>
        ) : (
          <ul className="divide-y divide-zinc-100">
            {invoices.map((inv) => (
              <li key={inv.id} className="flex items-center gap-3 px-4 py-3.5">
                <button
                  type="button"
                  onClick={() => setViewing(inv)}
                  className="min-w-0 flex-1 text-left"
                  title="View invoice"
                >
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="font-mono text-[12.5px] font-medium text-zinc-900">{inv.number}</span>
                    <span className="text-[14px] font-medium text-zinc-900">{money(inv.amountCents)}</span>
                    <Pill kind="invoice" status={inv.status} />
                  </div>
                  <div className="mt-0.5 truncate text-[12.5px] text-zinc-500">
                    {inv.clientName}
                    {inv.projectTitle ? ` · ${inv.projectTitle}` : ""}
                    {inv.dueAt ? ` · due ${inv.dueAt}` : ""}
                  </div>
                </button>
                <div className="flex shrink-0 items-center gap-1">
                  <select
                    value={inv.status}
                    disabled={busyId === inv.id}
                    onChange={(e) => setStatus(inv.id, e.target.value)}
                    aria-label="Status"
                    className="h-8 rounded-md border border-zinc-300 bg-white px-2 text-[12px] text-zinc-600 outline-none focus:border-brand"
                  >
                    <option value="draft">Draft</option>
                    <option value="sent">Sent</option>
                    <option value="paid">Paid</option>
                    <option value="overdue">Overdue</option>
                  </select>
                  <button type="button" onClick={() => setViewing(inv)} className="rounded-md px-2.5 py-1 text-[12.5px] font-medium text-zinc-600 hover:bg-zinc-100">
                    View
                  </button>
                  <button type="button" onClick={() => setEditing(inv)} className="rounded-md px-2.5 py-1 text-[12.5px] font-medium text-zinc-600 hover:bg-zinc-100">
                    Edit
                  </button>
                  <button
                    type="button"
                    disabled={busyId === inv.id}
                    onClick={() => remove(inv)}
                    className="rounded-md px-2 py-1 text-[12.5px] text-zinc-400 hover:bg-red-50 hover:text-red-600 disabled:opacity-50"
                    title="Delete"
                  >
                    ✕
                  </button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </Card>

      {viewing ? (
        <InvoiceView
          inv={viewing}
          client={clientFor(viewing)}
          onClose={() => setViewing(null)}
          onEdit={() => { setEditing(viewing); setViewing(null); }}
        />
      ) : null}

      {editing ? (
        <InvoiceForm
          initial={editing === "new" ? null : editing}
          clients={clients}
          projects={projects}
          nextNumber={`INV-${String(invoices.length + 1).padStart(3, "0")}`}
          onClose={() => setEditing(null)}
          onSaved={load}
        />
      ) : null}
    </div>
  );
}

// Assemble the PDF payload from an invoice + its client + the studio config,
// then download it. The renderer is imported lazily here so it never lands in
// SSR or the initial bundle.
async function downloadInvoicePdf(inv: InvoiceRow, client?: ClientRow) {
  const { buildInvoiceBlob } = await import("@/lib/invoice-pdf");
  const blob = await buildInvoiceBlob({
    number: inv.number,
    status: inv.status,
    issuedAt: inv.issuedAt,
    dueAt: inv.dueAt,
    notes: inv.notes,
    projectTitle: inv.projectTitle,
    items: parseLineItems(inv.lineItems),
    amountCents: inv.amountCents,
    billTo: { name: inv.clientName, company: client?.company, email: client?.email },
    studio: {
      name: siteConfig.brand.name,
      addressLines: siteConfig.billing.addressLines,
      paymentTerms: siteConfig.billing.paymentTerms,
      footerNote: siteConfig.billing.footerNote,
      brandColor: siteConfig.colors.brand,
    },
  });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = `${inv.number}.pdf`;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 2000);
}

// The full invoice document — an on-screen preview that mirrors the PDF, with
// Download PDF + Edit in the toolbar.
function InvoiceView({
  inv,
  client,
  onClose,
  onEdit,
}: {
  inv: InvoiceRow;
  client?: ClientRow;
  onClose: () => void;
  onEdit: () => void;
}) {
  const [downloading, setDownloading] = useState(false);
  const items = parseLineItems(inv.lineItems);
  const rows = items.length > 0 ? items : [{ description: "Services", quantity: 1, unitCents: inv.amountCents }];
  const { brand, billing } = siteConfig;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  async function download() {
    setDownloading(true);
    try {
      await downloadInvoicePdf(inv, client);
    } catch {
      /* generation is best-effort; the dashboard stays usable */
    } finally {
      setDownloading(false);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-black/40 p-4 sm:p-8" onMouseDown={onClose}>
      <div
        onMouseDown={(e) => e.stopPropagation()}
        className="my-auto w-full max-w-2xl overflow-hidden rounded-2xl bg-white shadow-[0_24px_64px_-16px_rgba(0,0,0,0.4)]"
      >
        {/* Toolbar */}
        <div className="flex items-center justify-between border-b border-zinc-100 px-5 py-3">
          <h2 className="font-mono text-[14px] font-semibold text-zinc-900">{inv.number}</h2>
          <div className="flex items-center gap-2">
            <button type="button" onClick={onEdit} className="rounded-lg px-3 py-1.5 text-[13px] font-medium text-zinc-600 hover:bg-zinc-100">
              Edit
            </button>
            <button
              type="button"
              onClick={download}
              disabled={downloading}
              className="rounded-lg bg-brand px-3.5 py-1.5 text-[13px] font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-50"
            >
              {downloading ? "Preparing…" : "Download PDF"}
            </button>
            <button type="button" onClick={onClose} className="rounded-md px-2 py-1 text-zinc-400 hover:bg-zinc-100 hover:text-zinc-700">✕</button>
          </div>
        </div>

        {/* Document */}
        <div className="max-h-[75vh] overflow-y-auto px-8 py-8 text-[13px] text-zinc-700">
          <div className="flex items-start justify-between gap-6">
            <div>
              <div className="text-[17px] font-semibold text-zinc-900">{brand.name}</div>
              {billing.addressLines.map((l, i) => (
                <div key={i} className="text-[12px] text-zinc-500">{l}</div>
              ))}
            </div>
            <div className="text-right">
              <div className="font-mono text-[11px] font-semibold uppercase tracking-[0.16em] text-brand">Invoice</div>
              <div className="font-mono text-[14px] font-semibold text-zinc-900">{inv.number}</div>
              <div className="mt-1"><Pill kind="invoice" status={inv.status} /></div>
            </div>
          </div>

          <div className="my-6 border-t border-zinc-200" />

          <div className="flex items-start justify-between gap-6">
            <div>
              <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-zinc-400">Bill to</div>
              <div className="mt-1 font-medium text-zinc-900">{inv.clientName}</div>
              {client?.company ? <div className="text-[12px] text-zinc-500">{client.company}</div> : null}
              {client?.email ? <div className="text-[12px] text-zinc-500">{client.email}</div> : null}
            </div>
            <div className="w-48 space-y-1 text-[12px]">
              {inv.projectTitle ? <MetaLine label="Project" value={inv.projectTitle} /> : null}
              {inv.issuedAt ? <MetaLine label="Issued" value={inv.issuedAt} /> : null}
              {inv.dueAt ? <MetaLine label="Due" value={inv.dueAt} /> : null}
              <MetaLine label="Terms" value={billing.paymentTerms} />
            </div>
          </div>

          <table className="mt-7 w-full text-[12.5px]">
            <thead>
              <tr className="border-b border-zinc-900 text-[10px] uppercase tracking-wide text-zinc-500">
                <th className="py-1.5 text-left font-semibold">Description</th>
                <th className="py-1.5 text-right font-semibold">Qty</th>
                <th className="py-1.5 text-right font-semibold">Unit</th>
                <th className="py-1.5 text-right font-semibold">Amount</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((it, i) => (
                <tr key={i} className="border-b border-zinc-100">
                  <td className="py-2 pr-3">{it.description}</td>
                  <td className="py-2 text-right tabular-nums">{it.quantity}</td>
                  <td className="py-2 text-right tabular-nums">{money(it.unitCents)}</td>
                  <td className="py-2 text-right tabular-nums">{money(Math.round(it.quantity * it.unitCents))}</td>
                </tr>
              ))}
            </tbody>
          </table>

          <div className="mt-4 flex justify-end">
            <div className="flex w-56 items-baseline justify-between">
              <span className="text-[13px] font-semibold text-zinc-900">Total due</span>
              <span className="text-[18px] font-semibold tabular-nums text-brand">{money(inv.amountCents)}</span>
            </div>
          </div>

          {inv.notes ? (
            <div className="mt-8">
              <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-zinc-400">Notes</div>
              <p className="mt-1 whitespace-pre-wrap text-[12.5px] text-zinc-600">{inv.notes}</p>
            </div>
          ) : null}

          <p className="mt-10 text-center text-[11px] text-zinc-400">{billing.footerNote}</p>
        </div>
      </div>
    </div>
  );
}

function MetaLine({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-3">
      <span className="text-zinc-400">{label}</span>
      <span className="font-medium text-zinc-700">{value}</span>
    </div>
  );
}

// One editable line in the invoice form — strings while editing, converted to
// { quantity, unitCents } on save.
interface ItemDraft {
  description: string;
  quantity: string;
  unit: string; // dollars, as typed
}

function initialItems(initial: InvoiceRow | null): ItemDraft[] {
  const parsed = initial ? parseLineItems(initial.lineItems) : [];
  if (parsed.length > 0) {
    return parsed.map((it) => ({
      description: it.description,
      quantity: String(it.quantity),
      unit: (it.unitCents / 100).toFixed(2),
    }));
  }
  // A legacy invoice (amount, no items) seeds one line at its total.
  if (initial && initial.amountCents > 0) {
    return [{ description: "Services", quantity: "1", unit: (initial.amountCents / 100).toFixed(2) }];
  }
  return [{ description: "", quantity: "1", unit: "" }];
}

function draftToItem(d: ItemDraft): InvoiceLineItem {
  return {
    description: d.description.trim(),
    quantity: Number(d.quantity) || 0,
    unitCents: Math.max(0, Math.round((parseFloat(d.unit) || 0) * 100)),
  };
}

function InvoiceForm({
  initial,
  clients,
  projects,
  nextNumber,
  onClose,
  onSaved,
}: {
  initial: InvoiceRow | null;
  clients: ClientRow[];
  projects: ProjectRow[];
  nextNumber: string;
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const [f, setF] = useState({
    number: initial?.number ?? nextNumber,
    clientId: initial?.clientId ?? clients[0]?.id ?? "",
    projectId: initial?.projectId ?? "",
    status: initial?.status ?? "draft",
    issuedAt: initial?.issuedAt ?? "",
    dueAt: initial?.dueAt ?? "",
    notes: initial?.notes ?? "",
  });
  const [items, setItems] = useState<ItemDraft[]>(() => initialItems(initial));
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const set = <K extends keyof typeof f>(k: K, v: (typeof f)[K]) => setF((s) => ({ ...s, [k]: v }) as typeof f);

  const setItem = (i: number, patch: Partial<ItemDraft>) =>
    setItems((arr) => arr.map((it, idx) => (idx === i ? { ...it, ...patch } : it)));
  const addItem = () => setItems((arr) => [...arr, { description: "", quantity: "1", unit: "" }]);
  const removeItem = (i: number) => setItems((arr) => (arr.length > 1 ? arr.filter((_, idx) => idx !== i) : arr));

  const totalCents = lineItemsTotal(items.map(draftToItem));

  async function save() {
    if (!f.clientId) { setErr("Pick a client."); return; }
    if (!f.number.trim()) { setErr("An invoice number is required."); return; }
    const lineItems = items.map(draftToItem).filter((it) => it.description.length > 0 || it.unitCents > 0);
    setSaving(true);
    setErr(null);
    try {
      await callFn("upsertInvoice", {
        id: initial?.id,
        number: f.number.trim(),
        clientId: f.clientId,
        projectId: f.projectId || undefined,
        lineItems,
        amountCents: totalCents,
        status: f.status,
        issuedAt: f.issuedAt || undefined,
        dueAt: f.dueAt || undefined,
        notes: f.notes.trim() || undefined,
      });
      await onSaved();
      onClose();
    } catch (e) {
      setErr(e instanceof Error ? e.message : "Couldn't save — try again.");
      setSaving(false);
    }
  }

  const noClients = clients.length === 0;

  return (
    <Modal title={initial ? "Edit invoice" : "New invoice"} onClose={onClose} onSubmit={save} saving={saving} error={err}>
      {noClients ? (
        <p className="rounded-lg bg-amber-50 px-3 py-2 text-[13px] text-amber-800">
          Add a client first — an invoice needs someone to bill.
        </p>
      ) : null}
      <div className="grid gap-3 sm:grid-cols-2">
        <Field label="Number" required>
          <input value={f.number} onChange={(e) => set("number", e.target.value)} className={inputCls + " font-mono"} />
        </Field>
        <Field label="Client" required>
          <select value={f.clientId} onChange={(e) => set("clientId", e.target.value)} className={inputCls} disabled={noClients}>
            {noClients ? <option value="">No clients yet</option> : null}
            {clients.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name}{c.company ? ` · ${c.company}` : ""}
              </option>
            ))}
          </select>
        </Field>
      </div>

      {/* Line items */}
      <div>
        <span className="mb-1 block text-[12.5px] font-medium text-zinc-600">Line items</span>
        <div className="space-y-2">
          {items.map((it, i) => (
            <div key={i} className="flex items-center gap-2">
              <input
                value={it.description}
                onChange={(e) => setItem(i, { description: e.target.value })}
                placeholder="Description"
                className={inputBase + " min-w-0 flex-1"}
              />
              <input
                type="number"
                min={0}
                value={it.quantity}
                onChange={(e) => setItem(i, { quantity: e.target.value })}
                aria-label="Quantity"
                title="Quantity"
                className={inputBase + " w-14 shrink-0 text-center tabular-nums"}
              />
              <div className="relative w-28 shrink-0">
                <span className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-[13px] text-zinc-400">$</span>
                <input
                  type="number"
                  min={0}
                  step="0.01"
                  value={it.unit}
                  onChange={(e) => setItem(i, { unit: e.target.value })}
                  placeholder="0.00"
                  aria-label="Unit price"
                  title="Unit price"
                  className={inputBase + " w-full pl-6 text-right tabular-nums"}
                />
              </div>
              <button
                type="button"
                onClick={() => removeItem(i)}
                disabled={items.length === 1}
                className="shrink-0 rounded-md px-2 py-1 text-[13px] text-zinc-400 hover:bg-red-50 hover:text-red-600 disabled:opacity-30"
                title="Remove line"
              >
                ✕
              </button>
            </div>
          ))}
        </div>
        <div className="mt-2 flex items-center justify-between">
          <button type="button" onClick={addItem} className="text-[12.5px] font-medium text-brand hover:underline">
            + Add line
          </button>
          <span className="text-[13px] text-zinc-500">
            Total <span className="font-semibold tabular-nums text-zinc-900">{money(totalCents)}</span>
          </span>
        </div>
      </div>

      <Field label="Project" hint="Optional — ties the bill to a case study">
        <select value={f.projectId} onChange={(e) => set("projectId", e.target.value)} className={inputCls}>
          <option value="">None</option>
          {projects.map((p) => (
            <option key={p.id} value={p.id}>{p.title}</option>
          ))}
        </select>
      </Field>
      <div className="grid gap-3 sm:grid-cols-3">
        <Field label="Status">
          <select value={f.status} onChange={(e) => set("status", e.target.value)} className={inputCls}>
            <option value="draft">Draft</option>
            <option value="sent">Sent</option>
            <option value="paid">Paid</option>
            <option value="overdue">Overdue</option>
          </select>
        </Field>
        <Field label="Issued">
          <input type="date" value={f.issuedAt ?? ""} onChange={(e) => set("issuedAt", e.target.value)} className={inputCls} />
        </Field>
        <Field label="Due">
          <input type="date" value={f.dueAt ?? ""} onChange={(e) => set("dueAt", e.target.value)} className={inputCls} />
        </Field>
      </div>
      <Field label="Notes">
        <textarea value={f.notes} onChange={(e) => set("notes", e.target.value)} rows={2} className={inputCls + " resize-none py-2"} />
      </Field>
    </Modal>
  );
}

/* ============================== PRIMITIVES ============================= */

function PanelHead({ title, sub, action }: { title: string; sub: string; action?: React.ReactNode }) {
  return (
    <div className="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h1 className="text-xl font-semibold tracking-tight">{title}</h1>
        <p className="mt-1 max-w-xl text-sm text-zinc-500">{sub}</p>
      </div>
      {action}
    </div>
  );
}

function NewButton({ onClick, children }: { onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="shrink-0 rounded-lg bg-zinc-900 px-3.5 py-2 text-[13px] font-medium text-white transition-colors hover:bg-zinc-700"
    >
      + {children}
    </button>
  );
}

function Card({ title, count, children }: { title: string; count: number; children: React.ReactNode }) {
  return (
    <div className="rounded-xl border border-zinc-200 bg-white">
      <div className="border-b border-zinc-100 px-4 py-3 text-sm font-semibold text-zinc-900">
        {title} <span className="font-normal text-zinc-400">({count})</span>
      </div>
      {children}
    </div>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return <p className="p-8 text-center text-sm text-zinc-500">{children}</p>;
}

function Stat({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="rounded-xl border border-zinc-200 bg-white p-4">
      <div className="text-[11px] font-medium uppercase tracking-wide text-zinc-400">{label}</div>
      <div className="mt-1 text-2xl font-semibold tabular-nums text-zinc-900">{value}</div>
      {hint ? <div className="mt-0.5 text-[12px] text-zinc-400">{hint}</div> : null}
    </div>
  );
}

// One pill component for every status across the dashboard, tone-mapped by kind.
function Pill({ kind, status }: { kind: "inquiry" | "client" | "invoice"; status: string }) {
  const tones: Record<string, string> = {
    // inquiry
    new: "bg-amber-50 text-amber-700",
    booked: "bg-green-50 text-green-700",
    declined: "bg-zinc-100 text-zinc-400",
    // client
    prospect: "bg-blue-50 text-blue-700",
    active: "bg-green-50 text-green-700",
    past: "bg-zinc-100 text-zinc-500",
    // invoice
    draft: "bg-zinc-100 text-zinc-500",
    sent: "bg-blue-50 text-blue-700",
    paid: "bg-green-50 text-green-700",
    overdue: "bg-red-50 text-red-700",
  };
  const label = kind === "inquiry" && status === "new" ? "new lead" : status;
  return (
    <span className={"rounded-full px-2 py-0.5 text-[10px] font-medium capitalize " + (tones[status] ?? "bg-zinc-100 text-zinc-500")}>
      {label}
    </span>
  );
}

function IconToggle({
  on,
  disabled,
  title,
  onClick,
  children,
}: {
  on: boolean;
  disabled?: boolean;
  title: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      title={title}
      onClick={onClick}
      className={
        "rounded-md px-2 py-1 text-[13px] leading-none transition-colors disabled:opacity-50 " +
        (on ? "text-amber-500 hover:bg-amber-50" : "text-zinc-300 hover:bg-zinc-100 hover:text-zinc-500")
      }
    >
      {children}
    </button>
  );
}

function Switch({ label, checked, onChange }: { label: string; checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button type="button" onClick={() => onChange(!checked)} className="flex items-center gap-2 text-[13px] text-zinc-700">
      <span
        className={
          "relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors " +
          (checked ? "bg-brand" : "bg-zinc-300")
        }
      >
        <span className={"inline-block size-4 transform rounded-full bg-white transition-transform " + (checked ? "translate-x-4" : "translate-x-0.5")} />
      </span>
      {label}
    </button>
  );
}

// A simple centered modal with a sticky footer. Submits on the footer button or
// Enter; closes on the backdrop, the ✕, or Escape.
function Modal({
  title,
  onClose,
  onSubmit,
  saving,
  error,
  children,
}: {
  title: string;
  onClose: () => void;
  onSubmit: () => void;
  saving: boolean;
  error: string | null;
  children: React.ReactNode;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-black/30 p-4 sm:p-8" onMouseDown={onClose}>
      <form
        onMouseDown={(e) => e.stopPropagation()}
        onSubmit={(e) => { e.preventDefault(); onSubmit(); }}
        className="my-auto w-full max-w-lg rounded-2xl bg-white shadow-[0_24px_64px_-16px_rgba(0,0,0,0.35)]"
      >
        <div className="flex items-center justify-between border-b border-zinc-100 px-5 py-3.5">
          <h2 className="text-[15px] font-semibold text-zinc-900">{title}</h2>
          <button type="button" onClick={onClose} className="rounded-md px-2 py-1 text-zinc-400 hover:bg-zinc-100 hover:text-zinc-700">✕</button>
        </div>
        <div className="max-h-[70vh] space-y-3.5 overflow-y-auto px-5 py-4">{children}</div>
        <div className="flex items-center justify-between gap-3 border-t border-zinc-100 px-5 py-3.5">
          <span className="text-[12.5px] text-red-600">{error}</span>
          <div className="flex items-center gap-2">
            <button type="button" onClick={onClose} className="rounded-lg px-3.5 py-2 text-[13px] font-medium text-zinc-600 hover:bg-zinc-100">
              Cancel
            </button>
            <button type="submit" disabled={saving} className="rounded-lg bg-brand px-4 py-2 text-[13px] font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-50">
              {saving ? "Saving…" : "Save"}
            </button>
          </div>
        </div>
      </form>
    </div>
  );
}

function Field({ label, hint, required, children }: { label: string; hint?: string; required?: boolean; children: React.ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1 block text-[12.5px] font-medium text-zinc-600">
        {label}
        {required ? <span className="text-brand"> *</span> : null}
        {hint ? <span className="ml-1 font-normal text-zinc-400">— {hint}</span> : null}
      </span>
      {children}
    </label>
  );
}

// Base field styles WITHOUT a width, so flex rows (the invoice line items) can
// size each input themselves. `inputCls` is the full-width variant for the
// stacked form fields.
const inputBase =
  "h-9 rounded-lg border border-zinc-300 bg-white px-3 text-[14px] text-zinc-900 outline-none transition placeholder:text-zinc-400 focus:border-brand focus:ring-2 focus:ring-brand/20";
const inputCls = inputBase + " w-full";

function OwnerOnly({ email }: { email: string }) {
  return (
    <div className="rounded-xl border border-dashed border-zinc-300 px-6 py-12 text-center">
      <h1 className="text-lg font-semibold">This dashboard is owner-only</h1>
      <p className="mx-auto mt-2 max-w-md text-sm text-zinc-500">
        You&apos;re signed in as <span className="font-medium text-zinc-700">{email || "this account"}</span>.
        Only the studio owner can see the back-office. Set{" "}
        <code className="rounded bg-zinc-100 px-1.5 py-0.5 text-[12px]">PYLON_OWNER_EMAIL={email || "you@studio.com"}</code>{" "}
        in your <code className="rounded bg-zinc-100 px-1.5 py-0.5 text-[12px]">.env</code>, restart, and reload —
        or sign in with the owner account.
      </p>
    </div>
  );
}

export function UserMenu({ email }: { email: string }) {
  const { signOut } = useAuth();
  const initial = (email.trim()[0] || "?").toUpperCase();
  async function onSignOut() {
    await signOut();
    window.location.assign("/");
  }
  return (
    <details className="group relative">
      <summary className="flex size-8 cursor-pointer select-none list-none items-center justify-center rounded-full bg-zinc-900 text-[12px] font-semibold text-white marker:hidden [&::-webkit-details-marker]:hidden">
        {initial}
      </summary>
      <div className="absolute right-0 top-full z-40 mt-2 w-56 overflow-hidden rounded-xl border border-zinc-200 bg-white py-1 shadow-[0_16px_48px_-16px_rgba(0,0,0,0.25)]">
        <div className="border-b border-zinc-100 px-3 py-2">
          <div className="truncate text-[13px] font-medium text-zinc-900">{email || "Signed in"}</div>
        </div>
        <button
          type="button"
          onClick={onSignOut}
          className="flex w-full items-center px-3 py-2 text-left text-[13px] text-zinc-700 transition-colors hover:bg-zinc-50"
        >
          Sign out
        </button>
      </div>
    </details>
  );
}

function Skeleton({ compact }: { compact?: boolean }) {
  return (
    <div className="space-y-8">
      {!compact ? <div className="h-6 w-28 animate-pulse rounded bg-zinc-100" /> : null}
      <div className="grid gap-4 sm:grid-cols-3">
        {[0, 1, 2].map((i) => (
          <div key={i} className="h-20 animate-pulse rounded-xl bg-zinc-100" />
        ))}
      </div>
      <div className="h-48 animate-pulse rounded-xl bg-zinc-100" />
    </div>
  );
}
