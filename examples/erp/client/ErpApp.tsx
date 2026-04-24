/**
 * Dallas Door Designs — custom residential entry doors, designed, fabricated,
 *
 * Multi-tenant: every row except User + Organization is scoped to an org.
 * Session carries an active tenantId via /api/auth/select-org so the
 * policy engine can gate reads on `auth.tenantId == data.orgId`.
 */

import React, { useEffect, useMemo, useRef, useState } from "react";
import {
  init,
  db,
  callFn,
  configureClient,
  storageKey,
  useRoom,
  useSession,
  type AggregateSpec,
} from "@pylonsync/react";

// Set VITE_PYLON_URL in Vercel (e.g. https://pylon-erp.fly.dev) to point
// the deployed frontend at the deployed backend; local dev falls back to
// localhost:4321 if the env var isn't set.
const BASE_URL = import.meta.env.VITE_PYLON_URL ?? "http://localhost:4321";
// Namespace so the ERP doesn't inherit auth/replica state from the chat
// demo (or any other Pylon app) when they share a browser origin.
init({ baseUrl: BASE_URL, appName: "erp" });
configureClient({ baseUrl: BASE_URL, appName: "erp" });

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type User = {
  id: string;
  email: string;
  displayName: string;
  avatarColor: string;
};

type Organization = {
  id: string;
  name: string;
  slug: string;
  billingEmail?: string | null;
  createdBy: string;
  createdAt: string;
};

type OrgMember = {
  id: string;
  userId: string;
  orgId: string;
  role: string;
  joinedAt: string;
};

type OrgInvite = {
  id: string;
  orgId: string;
  email: string;
  role: string;
  invitedBy: string;
  createdAt: string;
  acceptedAt?: string | null;
};

type Customer = {
  id: string;
  orgId: string;
  name: string;
  email?: string | null;
  phone?: string | null;
  company?: string | null;
  createdAt: string;
};

type Product = {
  id: string;
  orgId: string;
  name: string;
  category: string;
  sku?: string | null;
  basePrice: number;
  unit: string;
  active: boolean | number;
  leadTimeDays?: number | null;
  createdAt: string;
};

type Material = {
  id: string;
  orgId: string;
  name: string;
  sku?: string | null;
  unit: string;
  stockQty: number;
  reorderPoint: number;
  costPerUnit: number;
  supplier?: string | null;
  createdAt: string;
};

type Order = {
  id: string;
  orgId: string;
  customerId: string;
  number: string;
  status: string;
  subtotal: number;
  tax: number;
  total: number;
  dueDate?: string | null;
  shippedAt?: string | null;
  deliveredAt?: string | null;
  cancelledAt?: string | null;
  createdAt: string;
};

type OrderLine = {
  id: string;
  orgId: string;
  orderId: string;
  productId: string;
  description: string;
  qty: number;
  unitPrice: number;
  lineTotal: number;
  productionStatus: string;
  sortOrder: number;
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function initials(name: string | undefined): string {
  if (!name) return "·";
  const parts = name.trim().split(/\s+/);
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
}

function money(n: number): string {
  return n.toLocaleString(undefined, {
    style: "currency",
    currency: "USD",
    maximumFractionDigits: 2,
  });
}

function formatDate(iso: string): string {
  return new Date(iso).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

// Status values reuse the underlying Order state machine but the labels
// read as a residential door project pipeline: Designed → Fabrication →
// Finishing → Ready to install → Installed.
const ORDER_STATUS_STYLE: Record<
  string,
  { label: string; className: string }
> = {
  confirmed: { label: "Designed", className: "pill-accent" },
  in_production: { label: "In fabrication", className: "pill-warning" },
  ready: { label: "Finishing", className: "pill-accent" },
  shipped: { label: "Ready to install", className: "pill-success" },
  delivered: { label: "Installed", className: "pill-success" },
  cancelled: { label: "Cancelled", className: "pill-danger" },
};

// ---------------------------------------------------------------------------
// Filter bar — Attio/Linear-style chip filters
// ---------------------------------------------------------------------------

/**
 * Describes one filterable column. `kind` drives which operators + editor
 * UI show up. `options` supplies the `enum` choices (rendered as a
 * checklist) — omit for free-form strings.
 */
type FilterFieldKind = "string" | "number" | "date" | "enum" | "boolean";
type FilterField = {
  key: string;
  label: string;
  kind: FilterFieldKind;
  options?: { value: string; label: string }[];
};

/** One active filter condition. Persisted shape — easy to hydrate from URL. */
type FilterCondition = {
  field: string;
  op: string;
  // For multi-value ops like "is any of", value is an array. Otherwise
  // scalar (string | number | boolean) or null (for is-empty).
  value: unknown;
};

type FilterState = {
  search: string;
  conditions: FilterCondition[];
};

const EMPTY_FILTER: FilterState = { search: "", conditions: [] };

const OPS_BY_KIND: Record<FilterFieldKind, { op: string; label: string }[]> = {
  string: [
    { op: "eq", label: "is" },
    { op: "neq", label: "is not" },
    { op: "contains", label: "contains" },
    { op: "notContains", label: "does not contain" },
    { op: "startsWith", label: "starts with" },
    { op: "empty", label: "is empty" },
    { op: "notEmpty", label: "is not empty" },
  ],
  number: [
    { op: "eq", label: "=" },
    { op: "neq", label: "≠" },
    { op: "gt", label: ">" },
    { op: "gte", label: "≥" },
    { op: "lt", label: "<" },
    { op: "lte", label: "≤" },
  ],
  date: [
    { op: "after", label: "after" },
    { op: "before", label: "before" },
    { op: "between", label: "between" },
    { op: "relative", label: "in the last" },
  ],
  enum: [
    { op: "eq", label: "is" },
    { op: "in", label: "is any of" },
    { op: "neq", label: "is not" },
  ],
  boolean: [
    { op: "eq", label: "is" },
  ],
};

/**
 * Translate UI filter state into the server's `$` operator filter DSL.
 * Only the pieces that map 1:1 go server-side; the rest (`contains`,
 * `empty`) we handle client-side after the initial pull, since that data
 * is already in the sync replica.
 *
 * Returns `{ where }` suitable for `db.useQuery(entity, { where })`.
 */
function buildQueryFilter(
  fields: FilterField[],
  state: FilterState,
): Record<string, unknown> {
  const where: Record<string, unknown> = {};
  for (const cond of state.conditions) {
    const field = fields.find((f) => f.key === cond.field);
    if (!field) continue;
    switch (cond.op) {
      case "eq":
        where[cond.field] = cond.value;
        break;
      case "neq":
        where[cond.field] = { $not: cond.value };
        break;
      case "gt":
        where[cond.field] = { $gt: Number(cond.value) };
        break;
      case "gte":
        where[cond.field] = { $gte: Number(cond.value) };
        break;
      case "lt":
        where[cond.field] = { $lt: Number(cond.value) };
        break;
      case "lte":
        where[cond.field] = { $lte: Number(cond.value) };
        break;
      case "contains":
      case "startsWith":
        // Server supports $like — but we apply client-side below for
        // instant feedback against the local replica. Leave the server
        // where untouched; applyFilterClient handles these.
        break;
      case "in":
        if (Array.isArray(cond.value) && cond.value.length > 0) {
          where[cond.field] = { $in: cond.value };
        }
        break;
      case "after":
        if (cond.value) where[cond.field] = { $gte: String(cond.value) };
        break;
      case "before":
        if (cond.value) where[cond.field] = { $lte: String(cond.value) };
        break;
      case "relative": {
        // value is "N:unit" — e.g. "7:day", "2:week"
        const [nRaw, unit] = String(cond.value).split(":");
        const n = Number(nRaw);
        if (!isFinite(n) || n <= 0) break;
        const ms: Record<string, number> = {
          day: 86400_000,
          week: 7 * 86400_000,
          month: 30 * 86400_000,
          year: 365 * 86400_000,
        };
        const delta = ms[unit] ?? 86400_000;
        const cutoff = new Date(Date.now() - n * delta).toISOString();
        where[cond.field] = { $gte: cutoff };
        break;
      }
    }
  }
  return where;
}

/**
 * Apply the purely-client-side conditions (contains, startsWith, empty,
 * search) after the sync replica returns rows. Keeps the UX instant.
 */
function applyFilterClient<T extends Record<string, unknown>>(
  rows: T[],
  fields: FilterField[],
  state: FilterState,
): T[] {
  let out = rows;
  if (state.search.trim()) {
    const q = state.search.trim().toLowerCase();
    const searchable = fields
      .filter((f) => f.kind === "string" || f.kind === "enum")
      .map((f) => f.key);
    out = out.filter((row) =>
      searchable.some((k) =>
        String(row[k] ?? "").toLowerCase().includes(q),
      ),
    );
  }
  for (const cond of state.conditions) {
    switch (cond.op) {
      case "contains": {
        const v = String(cond.value ?? "").toLowerCase();
        if (!v) break;
        out = out.filter((r) =>
          String(r[cond.field] ?? "").toLowerCase().includes(v),
        );
        break;
      }
      case "notContains": {
        const v = String(cond.value ?? "").toLowerCase();
        if (!v) break;
        out = out.filter(
          (r) => !String(r[cond.field] ?? "").toLowerCase().includes(v),
        );
        break;
      }
      case "startsWith": {
        const v = String(cond.value ?? "").toLowerCase();
        if (!v) break;
        out = out.filter((r) =>
          String(r[cond.field] ?? "").toLowerCase().startsWith(v),
        );
        break;
      }
      case "empty":
        out = out.filter(
          (r) => r[cond.field] === null || r[cond.field] === "" || r[cond.field] === undefined,
        );
        break;
      case "notEmpty":
        out = out.filter(
          (r) =>
            r[cond.field] !== null &&
            r[cond.field] !== "" &&
            r[cond.field] !== undefined,
        );
        break;
    }
  }
  return out;
}

function formatChipValue(
  field: FilterField,
  cond: FilterCondition,
): string {
  if (cond.op === "empty") return "empty";
  if (cond.op === "notEmpty") return "not empty";
  if (cond.op === "relative") {
    const [n, unit] = String(cond.value ?? "").split(":");
    return `last ${n || "?"} ${unit || "day"}${n === "1" ? "" : "s"}`;
  }
  if (cond.op === "in" && Array.isArray(cond.value)) {
    if (cond.value.length === 0) return "…";
    if (cond.value.length <= 2) {
      return cond.value
        .map((v) => {
          const opt = field.options?.find((o) => o.value === v);
          return opt?.label ?? String(v);
        })
        .join(", ");
    }
    return `${cond.value.length} selected`;
  }
  const raw = String(cond.value ?? "…");
  if (field.kind === "enum" && field.options) {
    const opt = field.options.find((o) => o.value === raw);
    if (opt) return opt.label;
  }
  return raw;
}

function FilterBar({
  fields,
  value,
  onChange,
}: {
  fields: FilterField[];
  value: FilterState;
  onChange: (next: FilterState) => void;
}) {
  const [addOpen, setAddOpen] = useState(false);
  const [editingIdx, setEditingIdx] = useState<number | null>(null);
  const addRef = useRef<HTMLDivElement>(null);

  // Click outside closes the "add" dropdown.
  useEffect(() => {
    if (!addOpen) return;
    const onDown = (e: MouseEvent) => {
      if (addRef.current && !addRef.current.contains(e.target as Node)) {
        setAddOpen(false);
      }
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [addOpen]);

  function addCondition(field: FilterField) {
    const firstOp = OPS_BY_KIND[field.kind][0];
    const initialValue =
      field.kind === "boolean"
        ? true
        : field.kind === "enum" && firstOp.op === "in"
          ? []
          : "";
    onChange({
      ...value,
      conditions: [
        ...value.conditions,
        { field: field.key, op: firstOp.op, value: initialValue },
      ],
    });
    setEditingIdx(value.conditions.length);
    setAddOpen(false);
  }

  function updateCondition(idx: number, patch: Partial<FilterCondition>) {
    onChange({
      ...value,
      conditions: value.conditions.map((c, i) =>
        i === idx ? { ...c, ...patch } : c,
      ),
    });
  }

  function removeCondition(idx: number) {
    onChange({
      ...value,
      conditions: value.conditions.filter((_, i) => i !== idx),
    });
  }

  return (
    <div className="filter-bar">
      <div className="filter-search">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" style={{ color: "var(--text-dim)" }}>
          <circle cx="11" cy="11" r="7" stroke="currentColor" strokeWidth="2" />
          <path d="M21 21l-4.35-4.35" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
        </svg>
        <input
          value={value.search}
          onChange={(e) => onChange({ ...value, search: e.target.value })}
          placeholder="Search…"
        />
      </div>
      {value.conditions.map((cond, i) => {
        const field = fields.find((f) => f.key === cond.field);
        if (!field) return null;
        const op = OPS_BY_KIND[field.kind].find((o) => o.op === cond.op);
        return (
          <FilterChip
            key={i}
            field={field}
            cond={cond}
            opLabel={op?.label ?? cond.op}
            editing={editingIdx === i}
            onEdit={() => setEditingIdx(i)}
            onCloseEdit={() => setEditingIdx(null)}
            onChange={(patch) => updateCondition(i, patch)}
            onRemove={() => removeCondition(i)}
          />
        );
      })}
      <div className="popover-wrap" ref={addRef}>
        <button className="filter-add" onClick={() => setAddOpen((v) => !v)}>
          <IconPlus /> Filter
        </button>
        {addOpen && (
          <div className="dropdown">
            <div className="dropdown-section">Filter by</div>
            {fields.map((f) => (
              <div
                key={f.key}
                className="dropdown-item"
                onClick={() => addCondition(f)}
              >
                {f.label}
              </div>
            ))}
          </div>
        )}
      </div>
      {value.conditions.length > 0 && (
        <button
          className="btn btn-ghost"
          style={{ padding: "4px 10px", fontSize: 12 }}
          onClick={() => onChange({ ...value, conditions: [] })}
        >
          Clear
        </button>
      )}
    </div>
  );
}

function FilterChip({
  field,
  cond,
  opLabel,
  editing,
  onEdit,
  onCloseEdit,
  onChange,
  onRemove,
}: {
  field: FilterField;
  cond: FilterCondition;
  opLabel: string;
  editing: boolean;
  onEdit: () => void;
  onCloseEdit: () => void;
  onChange: (patch: Partial<FilterCondition>) => void;
  onRemove: () => void;
}) {
  const wrapRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!editing) return;
    const onDown = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        onCloseEdit();
      }
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [editing, onCloseEdit]);

  const ops = OPS_BY_KIND[field.kind];
  const [menu, setMenu] = useState<"op" | "val" | null>(null);
  const takesValue = cond.op !== "empty" && cond.op !== "notEmpty";

  return (
    <div className="filter-chip" ref={wrapRef}>
      <div className="filter-chip-seg key" title={field.label}>
        {field.label}
      </div>
      <div className="popover-wrap" style={{ display: "inline-block" }}>
        <div
          className="filter-chip-seg op"
          onClick={() => setMenu(menu === "op" ? null : "op")}
        >
          {opLabel}
        </div>
        {menu === "op" && (
          <div className="dropdown">
            {ops.map((o) => (
              <div
                key={o.op}
                className={
                  "dropdown-item" + (o.op === cond.op ? " selected" : "")
                }
                onClick={() => {
                  // Reset value when switching between scalar and array ops.
                  const nextValue =
                    o.op === "in" && !Array.isArray(cond.value)
                      ? []
                      : o.op !== "in" && Array.isArray(cond.value)
                        ? ""
                        : o.op === "empty" || o.op === "notEmpty"
                          ? null
                          : cond.value;
                  onChange({ op: o.op, value: nextValue });
                  setMenu(null);
                }}
              >
                {o.label}
              </div>
            ))}
          </div>
        )}
      </div>
      {takesValue && (
        <div className="popover-wrap" style={{ display: "inline-block" }}>
          <div
            className="filter-chip-seg val"
            onClick={() => setMenu(menu === "val" ? null : "val")}
          >
            {formatChipValue(field, cond)}
          </div>
          {menu === "val" && (
            <FilterValueEditor
              field={field}
              cond={cond}
              onChange={(v) => onChange({ value: v })}
              onClose={() => setMenu(null)}
            />
          )}
        </div>
      )}
      <button
        className="filter-chip-remove"
        onClick={onRemove}
        aria-label="Remove filter"
        title="Remove"
      >
        <svg width="10" height="10" viewBox="0 0 24 24" fill="none">
          <path
            d="M18 6L6 18M6 6l12 12"
            stroke="currentColor"
            strokeWidth="2.4"
            strokeLinecap="round"
          />
        </svg>
      </button>
    </div>
  );
}

function FilterValueEditor({
  field,
  cond,
  onChange,
  onClose,
}: {
  field: FilterField;
  cond: FilterCondition;
  onChange: (v: unknown) => void;
  onClose: () => void;
}) {
  if (field.kind === "enum" && field.options) {
    if (cond.op === "in") {
      const selected = Array.isArray(cond.value) ? (cond.value as string[]) : [];
      const toggle = (v: string) => {
        const next = selected.includes(v)
          ? selected.filter((s) => s !== v)
          : [...selected, v];
        onChange(next);
      };
      return (
        <div className="dropdown">
          {field.options.map((o) => {
            const on = selected.includes(o.value);
            return (
              <div
                key={o.value}
                className={"dropdown-item" + (on ? " selected" : "")}
                onClick={() => toggle(o.value)}
              >
                <span style={{ width: 12 }}>{on ? "✓" : ""}</span>
                {o.label}
              </div>
            );
          })}
        </div>
      );
    }
    return (
      <div className="dropdown">
        {field.options.map((o) => (
          <div
            key={o.value}
            className={
              "dropdown-item" + (cond.value === o.value ? " selected" : "")
            }
            onClick={() => {
              onChange(o.value);
              onClose();
            }}
          >
            {o.label}
          </div>
        ))}
      </div>
    );
  }

  if (field.kind === "boolean") {
    return (
      <div className="dropdown">
        {[
          { v: true, label: "True" },
          { v: false, label: "False" },
        ].map((o) => (
          <div
            key={String(o.v)}
            className={
              "dropdown-item" + (cond.value === o.v ? " selected" : "")
            }
            onClick={() => {
              onChange(o.v);
              onClose();
            }}
          >
            {o.label}
          </div>
        ))}
      </div>
    );
  }

  if (field.kind === "date") {
    if (cond.op === "relative") {
      const [nRaw, unit] = String(cond.value ?? "7:day").split(":");
      return (
        <div className="dropdown" style={{ padding: 8 }}>
          <div style={{ display: "flex", gap: 6 }}>
            <input
              className="dropdown-input"
              type="number"
              min="1"
              style={{ width: 70 }}
              value={nRaw}
              onChange={(e) =>
                onChange(`${e.target.value || "1"}:${unit || "day"}`)
              }
            />
            <select
              className="select"
              style={{ flex: 1 }}
              value={unit || "day"}
              onChange={(e) => onChange(`${nRaw || "1"}:${e.target.value}`)}
            >
              <option value="day">days</option>
              <option value="week">weeks</option>
              <option value="month">months</option>
              <option value="year">years</option>
            </select>
          </div>
        </div>
      );
    }
    return (
      <div className="dropdown" style={{ padding: 8 }}>
        <input
          className="dropdown-input"
          type="date"
          value={String(cond.value ?? "")}
          onChange={(e) => onChange(e.target.value)}
        />
      </div>
    );
  }

  if (field.kind === "number") {
    return (
      <div className="dropdown" style={{ padding: 8 }}>
        <input
          className="dropdown-input"
          type="number"
          autoFocus
          value={String(cond.value ?? "")}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && onClose()}
        />
      </div>
    );
  }

  // String default
  return (
    <div className="dropdown" style={{ padding: 8 }}>
      <input
        className="dropdown-input"
        autoFocus
        value={String(cond.value ?? "")}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && onClose()}
        placeholder="Value…"
      />
    </div>
  );
}

// ---------------------------------------------------------------------------
// Root
// ---------------------------------------------------------------------------

export function ErpApp() {
  const [currentUser, setCurrentUser] = useState<User | null>(() => {
    try {
      const token = localStorage.getItem(storageKey("token"));
      const cached = localStorage.getItem(storageKey("user"));
      return token && cached ? (JSON.parse(cached) as User) : null;
    } catch {
      return null;
    }
  });
  // Server session is the single source of truth for the active tenant.
  // `useSession` fetches /api/auth/me, caches it on the engine, and
  // notifies on change — including the replica reset when the tenant
  // flips. We no longer mirror it in localStorage.
  const { tenantId: activeOrgId } = useSession(db.sync);
  const [page, setPage] = useState<Page>("dashboard");

  async function signOut() {
    const token = localStorage.getItem(storageKey("token"));
    localStorage.removeItem(storageKey("token"));
    localStorage.removeItem(storageKey("user"));
    if (token) {
      fetch(`${BASE_URL}/api/auth/session`, {
        method: "DELETE",
        headers: { Authorization: `Bearer ${token}` },
      }).catch(() => {});
    }
    try {
      indexedDB.deleteDatabase(`pylon_sync_erp`);
    } catch {}
    setCurrentUser(null);
    // Token changed → sync engine picks it up on next pull (identity-
    // change detection), but nudge it so useSession flips to anonymous
    // immediately instead of waiting for the reconnect cycle.
    await db.sync.notifySessionChanged();
  }

  async function selectOrg(orgId: string | null) {
    const token = localStorage.getItem(storageKey("token"));
    if (!token) return;
    const res = await fetch(`${BASE_URL}/api/auth/select-org`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${token}`,
      },
      body: JSON.stringify({ orgId }),
    });
    if (!res.ok) {
      const err = await res.json().catch(() => ({}));
      throw new Error(err.error?.message || `switch failed (${res.status})`);
    }
    // Server session just flipped. `notifySessionChanged` re-reads
    // /api/auth/me, trips the tenant-flip branch in the engine which
    // resets the replica, and notifies subscribers — so `useSession`
    // above re-renders with the new tenant and queries re-run.
    await db.sync.notifySessionChanged();
  }

  if (!currentUser) return <Login onReady={setCurrentUser} />;

  return (
    <OrgGate
      currentUser={currentUser}
      activeOrgId={activeOrgId}
      onSelectOrg={selectOrg}
      onSignOut={signOut}
    >
      {(org, myRole) => (
        <div className="app">
          <Topbar
            currentUser={currentUser}
            activeOrg={org}
            myRole={myRole}
            onSelectOrg={selectOrg}
            onSignOut={signOut}
          />
          <div className="body">
            <Sidebar page={page} onNavigate={setPage} />
            <main className="main">
              {page === "dashboard" && <Dashboard org={org} />}
              {page === "customers" && <CustomersPage org={org} />}
              {page === "products" && <ProductsPage org={org} />}
              {page === "inventory" && (
                <InventoryPage org={org} currentUser={currentUser} />
              )}
              {page === "orders" && <OrdersPage org={org} />}
              {page === "analytics" && <AnalyticsPage org={org} />}
              {page === "team" && (
                <TeamPage
                  org={org}
                  currentUser={currentUser}
                  myRole={myRole}
                />
              )}
            </main>
          </div>
        </div>
      )}
    </OrgGate>
  );
}

type Page =
  | "dashboard"
  | "customers"
  | "products"
  | "inventory"
  | "orders"
  | "analytics"
  | "team";

type DashboardPanel = {
  id: string;
  orgId: string;
  title: string;
  entity: string;
  chartKind: string;
  specJson: string;
  sortOrder: number;
  createdAt: string;
};

// ---------------------------------------------------------------------------
// OrgGate — shows onboarding / org-pick screen until the session has an
// active org. Passes the active Organization down to the app.
// ---------------------------------------------------------------------------

function OrgGate({
  currentUser,
  activeOrgId,
  onSelectOrg,
  onSignOut,
  children,
}: {
  currentUser: User;
  activeOrgId: string | null;
  onSelectOrg: (orgId: string | null) => Promise<void>;
  onSignOut: () => void;
  children: (org: Organization, myRole: string) => React.ReactNode;
}) {
  const { data: memberships } = db.useQuery<OrgMember>("OrgMember", {
    where: { userId: currentUser.id },
  });
  const { data: organizations } = db.useQuery<Organization>("Organization");
  const { data: invites } = db.useQuery<OrgInvite>("OrgInvite", {
    where: { email: currentUser.email },
  });

  const myOrgs = useMemo(() => {
    const byId = new Map<string, Organization>();
    for (const o of organizations ?? []) byId.set(o.id, o);
    const rows: { org: Organization; role: string }[] = [];
    for (const m of memberships ?? []) {
      const org = byId.get(m.orgId);
      if (org) rows.push({ org, role: m.role });
    }
    return rows.sort((a, b) => a.org.name.localeCompare(b.org.name));
  }, [memberships, organizations]);

  const pendingInvites = (invites ?? []).filter((i) => !i.acceptedAt);

  // Auto-select if we have a single org and no active.
  useEffect(() => {
    if (!activeOrgId && myOrgs.length === 1) {
      void onSelectOrg(myOrgs[0].org.id);
    }
  }, [activeOrgId, myOrgs]);

  const active = myOrgs.find((m) => m.org.id === activeOrgId);
  if (active) return <>{children(active.org, active.role)}</>;

  // No active org: show onboarding / picker.
  return (
    <OnboardingScreen
      currentUser={currentUser}
      myOrgs={myOrgs}
      pendingInvites={pendingInvites}
      onSelectOrg={onSelectOrg}
      onSignOut={onSignOut}
    />
  );
}

function OnboardingScreen({
  currentUser,
  myOrgs,
  pendingInvites,
  onSelectOrg,
  onSignOut,
}: {
  currentUser: User;
  myOrgs: { org: Organization; role: string }[];
  pendingInvites: OrgInvite[];
  onSelectOrg: (orgId: string) => Promise<void>;
  onSignOut: () => void;
}) {
  const [createOpen, setCreateOpen] = useState(myOrgs.length === 0 && pendingInvites.length === 0);
  const [busyInvite, setBusyInvite] = useState<string | null>(null);

  async function accept(inviteId: string) {
    setBusyInvite(inviteId);
    try {
      const res = await callFn<{ orgId: string }>("acceptInvite", { inviteId });
      await onSelectOrg(res.orgId);
    } catch (e) {
      alert((e as Error).message);
    } finally {
      setBusyInvite(null);
    }
  }

  return (
    <div className="split-screen">
      <div className="auth-panel" style={{ width: 520 }}>
        <div className="brand" style={{ marginBottom: 24 }}>
          <div className="brand-mark">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
              <path
                d="M4 7l8-4 8 4v10l-8 4-8-4V7z"
                stroke="white"
                strokeWidth="2"
                strokeLinejoin="round"
              />
              <path
                d="M12 11v10M4 7l8 4 8-4"
                stroke="white"
                strokeWidth="2"
                strokeLinejoin="round"
              />
            </svg>
          </div>
          Dallas Door Designs
        </div>
        <div className="auth-title">Hi, {currentUser.displayName}</div>
        <div className="auth-subtitle">
          Pick a workspace, accept an invite, or start a new one.
        </div>

        {pendingInvites.length > 0 && (
          <>
            <div
              style={{
                fontSize: 11,
                fontWeight: 600,
                letterSpacing: "0.08em",
                textTransform: "uppercase",
                color: "var(--text-dim)",
                marginTop: 8,
                marginBottom: 8,
              }}
            >
              Pending invites
            </div>
            {pendingInvites.map((inv) => (
              <InviteCard
                key={inv.id}
                invite={inv}
                busy={busyInvite === inv.id}
                onAccept={() => void accept(inv.id)}
              />
            ))}
          </>
        )}

        {myOrgs.length > 0 && (
          <>
            <div
              style={{
                fontSize: 11,
                fontWeight: 600,
                letterSpacing: "0.08em",
                textTransform: "uppercase",
                color: "var(--text-dim)",
                marginTop: 16,
                marginBottom: 8,
              }}
            >
              Your workspaces
            </div>
            {myOrgs.map(({ org, role }) => (
              <button
                key={org.id}
                className="popover-item"
                style={{
                  width: "100%",
                  marginBottom: 4,
                  padding: "10px 12px",
                }}
                onClick={() => void onSelectOrg(org.id)}
              >
                <div
                  className="avatar avatar-sm"
                  style={{ backgroundColor: "#c7d2fe" }}
                >
                  {initials(org.name)}
                </div>
                <div style={{ flex: 1, textAlign: "left" }}>
                  <div style={{ fontWeight: 500 }}>{org.name}</div>
                  <div style={{ fontSize: 11.5, color: "var(--text-dim)" }}>
                    {role}
                  </div>
                </div>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
                  <path
                    d="M9 6l6 6-6 6"
                    stroke="currentColor"
                    strokeWidth="2"
                    strokeLinecap="round"
                  />
                </svg>
              </button>
            ))}
          </>
        )}

        <div style={{ display: "flex", gap: 8, marginTop: 20 }}>
          <button className="btn btn-primary" onClick={() => setCreateOpen(true)}>
            Create new workspace
          </button>
          <button className="btn btn-ghost" onClick={onSignOut}>
            Sign out
          </button>
        </div>
      </div>
      {createOpen && (
        <CreateOrgModal
          onClose={() => setCreateOpen(false)}
          onCreated={(orgId) => {
            setCreateOpen(false);
            void onSelectOrg(orgId);
          }}
        />
      )}
    </div>
  );
}

function InviteCard({
  invite,
  busy,
  onAccept,
}: {
  invite: OrgInvite;
  busy: boolean;
  onAccept: () => void;
}) {
  const { data: org } = db.useQueryOne<Organization>(
    "Organization",
    invite.orgId,
  );
  const { data: inviter } = db.useQueryOne<User>("User", invite.invitedBy);
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 12,
        padding: 12,
        background: "var(--surface-hover)",
        border: "1px solid var(--border)",
        borderRadius: 10,
        marginBottom: 8,
      }}
    >
      <div
        className="avatar avatar-md"
        style={{ backgroundColor: "#c7d2fe" }}
      >
        {initials(org?.name)}
      </div>
      <div style={{ flex: 1 }}>
        <div style={{ fontWeight: 500 }}>{org?.name ?? "…"}</div>
        <div style={{ fontSize: 12, color: "var(--text-muted)" }}>
          {inviter?.displayName ?? "Someone"} invited you as{" "}
          <strong>{invite.role}</strong>
        </div>
      </div>
      <button
        className="btn btn-primary"
        onClick={onAccept}
        disabled={busy}
      >
        {busy ? "Accepting…" : "Accept"}
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Login
// ---------------------------------------------------------------------------

function Login({ onReady }: { onReady: (u: User) => void }) {
  const [email, setEmail] = useState("owner@acme.example");
  const [name, setName] = useState("Alex Owner");
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function go() {
    setLoading(true);
    setErr(null);
    try {
      const session = await fetch(`${BASE_URL}/api/auth/guest`, {
        method: "POST",
      }).then((r) => r.json());
      const token: string = session.token;
      localStorage.setItem(storageKey("token"), token);
      configureClient({ baseUrl: BASE_URL });
      const user = await callFn<User>("upsertUser", { email, displayName: name });
      await fetch(`${BASE_URL}/api/auth/upgrade`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${token}`,
        },
        body: JSON.stringify({ user_id: user.id }),
      });
      localStorage.setItem(storageKey("user"), JSON.stringify(user));
      void db.sync.pull();
      onReady(user);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="split-screen">
      <div className="auth-panel">
        <div className="brand" style={{ marginBottom: 20 }}>
          <div className="brand-mark">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
              <path
                d="M4 7l8-4 8 4v10l-8 4-8-4V7z"
                stroke="white"
                strokeWidth="2"
                strokeLinejoin="round"
              />
              <path
                d="M12 11v10M4 7l8 4 8-4"
                stroke="white"
                strokeWidth="2"
                strokeLinejoin="round"
              />
            </svg>
          </div>
          Dallas Door Designs
        </div>
        <div className="auth-title">Sign in</div>
        <div className="auth-subtitle">
          Custom-door projects, teams, and inventory in one place.
        </div>
        <label className="field">
          <span className="field-label">Email</span>
          <input
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            placeholder="you@shop.com"
            className="input"
            autoFocus
          />
        </label>
        <label className="field">
          <span className="field-label">Display name</span>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Alex"
            className="input"
            onKeyDown={(e) => e.key === "Enter" && go()}
          />
        </label>
        {err && <div className="error-text">{err}</div>}
        <button
          onClick={go}
          disabled={loading}
          className="btn btn-primary"
          style={{ width: "100%", marginTop: 8, padding: "10px 16px" }}
        >
          {loading ? "Signing in…" : "Continue"}
        </button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Topbar + Sidebar
// ---------------------------------------------------------------------------

function Topbar({
  currentUser,
  activeOrg,
  myRole,
  onSelectOrg,
  onSignOut,
}: {
  currentUser: User;
  activeOrg: Organization;
  myRole: string;
  onSelectOrg: (orgId: string | null) => Promise<void>;
  onSignOut: () => void;
}) {
  const [orgOpen, setOrgOpen] = useState(false);
  const [userOpen, setUserOpen] = useState(false);

  const { data: memberships } = db.useQuery<OrgMember>("OrgMember", {
    where: { userId: currentUser.id },
  });
  const { data: allOrgs } = db.useQuery<Organization>("Organization");
  const myOrgs = (memberships ?? [])
    .map((m) => (allOrgs ?? []).find((o) => o.id === m.orgId))
    .filter(Boolean) as Organization[];

  return (
    <header className="topbar">
      <div className="brand">
        <div className="brand-mark">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
            <path
              d="M4 7l8-4 8 4v10l-8 4-8-4V7z"
              stroke="white"
              strokeWidth="2"
              strokeLinejoin="round"
            />
            <path
              d="M12 11v10M4 7l8 4 8-4"
              stroke="white"
              strokeWidth="2"
              strokeLinejoin="round"
            />
          </svg>
        </div>
        Dallas Door Designs
      </div>
      <div className="popover-wrap">
        <button
          className="org-switcher"
          onClick={() => setOrgOpen((v) => !v)}
        >
          <div
            className="avatar avatar-xs"
            style={{ backgroundColor: "#c7d2fe" }}
          >
            {initials(activeOrg.name)}
          </div>
          {activeOrg.name}
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none">
            <path
              d="M6 9l6 6 6-6"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
            />
          </svg>
        </button>
        {orgOpen && (
          <div className="popover" style={{ left: 0 }}>
            <div className="popover-section">Switch workspace</div>
            {myOrgs.map((o) => (
              <div
                key={o.id}
                className={
                  "popover-item" + (o.id === activeOrg.id ? " selected" : "")
                }
                onClick={() => {
                  setOrgOpen(false);
                  void onSelectOrg(o.id);
                }}
              >
                <div
                  className="avatar avatar-xs"
                  style={{ backgroundColor: "#c7d2fe" }}
                >
                  {initials(o.name)}
                </div>
                {o.name}
              </div>
            ))}
            <div
              style={{ borderTop: "1px solid var(--border)", margin: "6px 0" }}
            />
            <div
              className="popover-item"
              onClick={() => {
                setOrgOpen(false);
                void onSelectOrg(null);
              }}
            >
              Switch to lobby…
            </div>
          </div>
        )}
      </div>
      <div className="spacer" />
      <OrgPresence activeOrg={activeOrg} currentUser={currentUser} />
      <div className="popover-wrap">
        <button
          className="user-chip"
          onClick={() => setUserOpen((v) => !v)}
        >
          <div
            className="avatar avatar-sm"
            style={{ backgroundColor: currentUser.avatarColor }}
          >
            {initials(currentUser.displayName)}
          </div>
          <div style={{ textAlign: "left" }}>
            <div style={{ fontSize: 12.5, fontWeight: 500 }}>
              {currentUser.displayName}
            </div>
            <div style={{ fontSize: 11, color: "var(--text-dim)" }}>
              {myRole}
            </div>
          </div>
        </button>
        {userOpen && (
          <div className="popover" style={{ right: 0 }}>
            <div className="popover-item" onClick={onSignOut}>
              Sign out
            </div>
          </div>
        )}
      </div>
    </header>
  );
}

/**
 * Live presence chip in the top bar — shows up to three avatars stacked
 * with an overflow pill, plus a popover listing everyone currently in the
 * org. Driven by `useRoom("org:<id>")`: every tab that mounts the Topbar
 * joins the same room, so presence is automatic across users AND across
 * multiple tabs from the same user (collapsed by user id in the popover).
 */
function OrgPresence({
  activeOrg,
  currentUser,
}: {
  activeOrg: Organization;
  currentUser: User;
}) {
  const { peers } = useRoom(`org:${activeOrg.id}`, currentUser.id, {
    initialPresence: {
      displayName: currentUser.displayName,
      avatarColor: currentUser.avatarColor,
    },
  });

  // Collapse duplicate tabs from the same user into a single entry — only
  // the most-recent join wins. Exclude self from the visible row so the
  // "3 online" label doesn't feel like it's counting the viewer twice.
  const others = useMemo(() => {
    const byId = new Map<string, (typeof peers)[number]>();
    for (const p of peers) {
      if (p.user_id === currentUser.id) continue;
      const existing = byId.get(p.user_id);
      if (!existing || p.joined_at > existing.joined_at) {
        byId.set(p.user_id, p);
      }
    }
    return Array.from(byId.values()).sort((a, b) => {
      const an =
        (a.data as { displayName?: string })?.displayName ?? "";
      const bn =
        (b.data as { displayName?: string })?.displayName ?? "";
      return an.localeCompare(bn);
    });
  }, [peers, currentUser.id]);

  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  const visible = others.slice(0, 3);
  const overflow = others.length - visible.length;

  return (
    <div className="popover-wrap" ref={wrapRef}>
      <button
        className="presence-stack"
        onClick={() => setOpen((v) => !v)}
        title={
          others.length === 0
            ? "No teammates online right now"
            : `${others.length} teammate${others.length === 1 ? "" : "s"} online`
        }
      >
        <div className="presence-stack-avatars">
          {/* Always show current user on the left so viewers see themselves. */}
          <div
            className="avatar avatar-xs"
            style={{ backgroundColor: currentUser.avatarColor }}
          >
            {initials(currentUser.displayName)}
          </div>
          {visible.map((p) => {
            const data = p.data as {
              displayName?: string;
              avatarColor?: string;
            };
            return (
              <div
                key={p.user_id}
                className="avatar avatar-xs"
                style={{ backgroundColor: data?.avatarColor || "#c7d2fe" }}
              >
                {initials(data?.displayName)}
              </div>
            );
          })}
          {overflow > 0 && (
            <span className="presence-more">+{overflow}</span>
          )}
        </div>
        <span className="presence-label">
          {others.length === 0
            ? "Only you"
            : `${others.length + 1} online`}
        </span>
      </button>
      {open && (
        <div className="popover" style={{ right: 0 }}>
          <div className="popover-section">In {activeOrg.name}</div>
          <div className="popover-item" style={{ cursor: "default" }}>
            <div
              className="avatar avatar-sm"
              style={{ backgroundColor: currentUser.avatarColor }}
            >
              {initials(currentUser.displayName)}
            </div>
            <div style={{ flex: 1 }}>
              {currentUser.displayName}
              <span
                style={{ color: "var(--text-dim)", marginLeft: 6, fontSize: 11 }}
              >
                you
              </span>
            </div>
            <span style={{ fontSize: 11, color: "var(--success)" }}>●</span>
          </div>
          {others.length === 0 ? (
            <div
              style={{
                padding: "10px 10px 6px",
                fontSize: 12,
                color: "var(--text-dim)",
              }}
            >
              No one else is online right now.
            </div>
          ) : (
            others.map((p) => {
              const data = p.data as {
                displayName?: string;
                avatarColor?: string;
              };
              return (
                <div
                  key={p.user_id}
                  className="popover-item"
                  style={{ cursor: "default" }}
                >
                  <div
                    className="avatar avatar-sm"
                    style={{ backgroundColor: data?.avatarColor || "#c7d2fe" }}
                  >
                    {initials(data?.displayName)}
                  </div>
                  <div style={{ flex: 1 }}>{data?.displayName ?? "Someone"}</div>
                  <span style={{ fontSize: 11, color: "var(--success)" }}>●</span>
                </div>
              );
            })
          )}
        </div>
      )}
    </div>
  );
}

function Sidebar({
  page,
  onNavigate,
}: {
  page: Page;
  onNavigate: (p: Page) => void;
}) {
  const items: { id: Page; label: string; icon: React.ReactNode }[] = [
    { id: "dashboard", label: "Dashboard", icon: <IconHome /> },
    { id: "orders", label: "Projects", icon: <IconClipboard /> },
    { id: "customers", label: "Customers", icon: <IconUsers /> },
    { id: "products", label: "Catalog", icon: <IconBox /> },
    { id: "inventory", label: "Shop floor", icon: <IconStack /> },
    { id: "analytics", label: "Analytics", icon: <IconChart /> },
    { id: "team", label: "Team", icon: <IconTeam /> },
  ];
  return (
    <nav className="nav">
      {items.map((it) => (
        <div
          key={it.id}
          className={"nav-item" + (page === it.id ? " active" : "")}
          onClick={() => onNavigate(it.id)}
          role="button"
          tabIndex={0}
        >
          {it.icon}
          {it.label}
        </div>
      ))}
    </nav>
  );
}

// ---------------------------------------------------------------------------
// Dashboard
// ---------------------------------------------------------------------------

function Dashboard({ org }: { org: Organization }) {
  const { data: orders } = db.useQuery<Order>("Order", {
    where: { orgId: org.id },
  });
  const { data: customers } = db.useQuery<Customer>("Customer", {
    where: { orgId: org.id },
  });
  const { data: materials } = db.useQuery<Material>("Material", {
    where: { orgId: org.id },
  });

  const openOrders = (orders ?? []).filter(
    (o) => o.status !== "delivered" && o.status !== "cancelled",
  );
  const revenueThisMonth = (orders ?? [])
    .filter((o) => {
      const d = new Date(o.createdAt);
      const now = new Date();
      return (
        d.getMonth() === now.getMonth() &&
        d.getFullYear() === now.getFullYear() &&
        o.status !== "cancelled"
      );
    })
    .reduce((sum, o) => sum + o.total, 0);
  const lowStock = (materials ?? []).filter(
    (m) => m.stockQty <= m.reorderPoint,
  );

  return (
    <>
      <div className="page-header">
        <div>
          <div className="page-title">Dashboard</div>
          <div className="page-subtitle">
            Snapshot of {org.name} — projects, shop floor, and customers.
          </div>
        </div>
      </div>
      <div className="kpi-grid">
        <div className="kpi">
          <div className="kpi-label">Open orders</div>
          <div className="kpi-value">{openOrders.length}</div>
          <div className="kpi-sub">
            {(orders ?? []).length} total lifetime
          </div>
        </div>
        <div className="kpi">
          <div className="kpi-label">Revenue this month</div>
          <div className="kpi-value">{money(revenueThisMonth)}</div>
          <div className="kpi-sub">Across confirmed + delivered</div>
        </div>
        <div className="kpi">
          <div className="kpi-label">Customers</div>
          <div className="kpi-value">{(customers ?? []).length}</div>
          <div className="kpi-sub">In {org.name}</div>
        </div>
        <div className="kpi">
          <div className="kpi-label">Low stock</div>
          <div
            className="kpi-value"
            style={{ color: lowStock.length > 0 ? "var(--warning)" : undefined }}
          >
            {lowStock.length}
          </div>
          <div className="kpi-sub">
            {lowStock.length === 0
              ? "All good"
              : "Materials at or below reorder point"}
          </div>
        </div>
      </div>
      {lowStock.length > 0 && (
        <div className="card" style={{ marginBottom: 20 }}>
          <div
            style={{
              fontSize: 13,
              fontWeight: 600,
              marginBottom: 8,
              color: "var(--warning)",
            }}
          >
            Materials needing reorder
          </div>
          <table className="table">
            <thead>
              <tr>
                <th>Name</th>
                <th>On hand</th>
                <th>Reorder at</th>
                <th>Supplier</th>
              </tr>
            </thead>
            <tbody>
              {lowStock.map((m) => (
                <tr key={m.id}>
                  <td>{m.name}</td>
                  <td>
                    {m.stockQty} {m.unit}
                  </td>
                  <td>
                    {m.reorderPoint} {m.unit}
                  </td>
                  <td>{m.supplier || "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}

// ---------------------------------------------------------------------------
// Customers
// ---------------------------------------------------------------------------

const CUSTOMER_FILTER_FIELDS: FilterField[] = [
  { key: "name", label: "Name", kind: "string" },
  { key: "company", label: "Company", kind: "string" },
  { key: "email", label: "Email", kind: "string" },
  { key: "phone", label: "Phone", kind: "string" },
  { key: "city", label: "City", kind: "string" },
  { key: "state", label: "State", kind: "string" },
  { key: "createdAt", label: "Added", kind: "date" },
];

function CustomersPage({ org }: { org: Organization }) {
  const [filter, setFilter] = useState<FilterState>(EMPTY_FILTER);
  const { data: customers } = db.useQuery<Customer>("Customer", {
    where: {
      orgId: org.id,
      ...buildQueryFilter(CUSTOMER_FILTER_FIELDS, filter),
    },
    orderBy: { createdAt: "desc" },
  });
  const filtered = useMemo(
    () => applyFilterClient(customers ?? [], CUSTOMER_FILTER_FIELDS, filter),
    [customers, filter],
  );
  const [addOpen, setAddOpen] = useState(false);

  return (
    <>
      <div className="page-header">
        <div>
          <div className="page-title">Customers</div>
          <div className="page-subtitle">
            Homeowners, builders, and designers you work with.
          </div>
        </div>
        <button className="btn btn-primary" onClick={() => setAddOpen(true)}>
          <IconPlus />
          Add customer
        </button>
      </div>
      <FilterBar
        fields={CUSTOMER_FILTER_FIELDS}
        value={filter}
        onChange={setFilter}
      />
      {filtered.length === 0 ? (
        <div className="empty">
          <div className="empty-title">
            {(customers ?? []).length === 0
              ? "No customers yet"
              : "No customers match these filters"}
          </div>
          <div className="empty-body">
            {(customers ?? []).length === 0
              ? "Add your first customer to start booking a project."
              : "Clear the filters or try a different search."}
          </div>
          <button className="btn btn-primary" onClick={() => setAddOpen(true)}>
            Add customer
          </button>
        </div>
      ) : (
        <table className="table">
          <thead>
            <tr>
              <th>Name</th>
              <th>Company</th>
              <th>Email</th>
              <th>Phone</th>
              <th>Added</th>
            </tr>
          </thead>
          <tbody>
            {filtered.map((c) => (
              <tr key={c.id}>
                <td style={{ fontWeight: 500 }}>{c.name}</td>
                <td style={{ color: "var(--text-muted)" }}>
                  {c.company || "—"}
                </td>
                <td style={{ color: "var(--text-muted)" }}>{c.email || "—"}</td>
                <td style={{ color: "var(--text-muted)" }}>{c.phone || "—"}</td>
                <td style={{ color: "var(--text-muted)" }}>
                  {formatDate(c.createdAt)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {addOpen && <AddCustomerModal onClose={() => setAddOpen(false)} />}
    </>
  );
}

function AddCustomerModal({ onClose }: { onClose: () => void }) {
  const [form, setForm] = useState({
    name: "",
    company: "",
    email: "",
    phone: "",
    notes: "",
  });
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function save() {
    setBusy(true);
    setErr(null);
    try {
      await callFn("createCustomer", {
        name: form.name,
        company: form.company || undefined,
        email: form.email || undefined,
        phone: form.phone || undefined,
        notes: form.notes || undefined,
      });
      onClose();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">New customer</div>
        <div className="modal-subtitle">Track who the work is for.</div>
        <div className="modal-body">
          <label className="field">
            <span className="field-label">Name</span>
            <input
              autoFocus
              className="input"
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
              placeholder="Jane Smith"
            />
          </label>
          <label className="field">
            <span className="field-label">Company</span>
            <input
              className="input"
              value={form.company}
              onChange={(e) => setForm({ ...form, company: e.target.value })}
              placeholder="Optional"
            />
          </label>
          <div className="row-2">
            <label className="field">
              <span className="field-label">Email</span>
              <input
                className="input"
                type="email"
                value={form.email}
                onChange={(e) => setForm({ ...form, email: e.target.value })}
              />
            </label>
            <label className="field">
              <span className="field-label">Phone</span>
              <input
                className="input"
                value={form.phone}
                onChange={(e) => setForm({ ...form, phone: e.target.value })}
              />
            </label>
          </div>
          <label className="field">
            <span className="field-label">Notes</span>
            <textarea
              className="textarea"
              value={form.notes}
              onChange={(e) => setForm({ ...form, notes: e.target.value })}
              placeholder="Anything worth remembering"
            />
          </label>
          {err && <div className="error-text">{err}</div>}
        </div>
        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>Cancel</button>
          <button
            className="btn btn-primary"
            disabled={busy || !form.name.trim()}
            onClick={() => void save()}
          >
            {busy ? "Saving…" : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Products
// ---------------------------------------------------------------------------

function ProductsPage({ org }: { org: Organization }) {
  const { data: products } = db.useQuery<Product>("Product", {
    where: { orgId: org.id },
    orderBy: { name: "asc" },
  });
  const [addOpen, setAddOpen] = useState(false);

  return (
    <>
      <div className="page-header">
        <div>
          <div className="page-title">Catalog</div>
          <div className="page-subtitle">
            Your door catalog — iron, wood, barn, pivot, and more.
          </div>
        </div>
        <button className="btn btn-primary" onClick={() => setAddOpen(true)}>
          <IconPlus />
          Add product
        </button>
      </div>
      {(products ?? []).length === 0 ? (
        <div className="empty">
          <div className="empty-title">No products yet</div>
          <div className="empty-body">
            Add doors and options from your catalog so you can quote them.
          </div>
          <button className="btn btn-primary" onClick={() => setAddOpen(true)}>
            Add product
          </button>
        </div>
      ) : (
        <table className="table">
          <thead>
            <tr>
              <th>Name</th>
              <th>Category</th>
              <th>SKU</th>
              <th>Base price</th>
              <th>Unit</th>
              <th>Lead time</th>
            </tr>
          </thead>
          <tbody>
            {(products ?? []).map((p) => (
              <tr key={p.id}>
                <td style={{ fontWeight: 500 }}>{p.name}</td>
                <td>
                  <span className="pill pill-gray">{p.category}</span>
                </td>
                <td style={{ color: "var(--text-muted)" }}>{p.sku || "—"}</td>
                <td style={{ fontVariantNumeric: "tabular-nums" }}>
                  {money(p.basePrice)}
                </td>
                <td style={{ color: "var(--text-muted)" }}>{p.unit}</td>
                <td style={{ color: "var(--text-muted)" }}>
                  {p.leadTimeDays ? `${p.leadTimeDays} days` : "—"}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {addOpen && <AddProductModal onClose={() => setAddOpen(false)} />}
    </>
  );
}

function AddProductModal({ onClose }: { onClose: () => void }) {
  const [form, setForm] = useState({
    name: "",
    category: "iron",
    basePrice: "",
    unit: "each",
    sku: "",
    description: "",
    leadTimeDays: "",
  });
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function save() {
    setBusy(true);
    setErr(null);
    try {
      await callFn("createProduct", {
        name: form.name,
        category: form.category,
        basePrice: parseFloat(form.basePrice) || 0,
        unit: form.unit,
        sku: form.sku || undefined,
        description: form.description || undefined,
        leadTimeDays: form.leadTimeDays
          ? parseInt(form.leadTimeDays, 10)
          : undefined,
      });
      onClose();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">New product</div>
        <div className="modal-subtitle">Add an item to your catalog.</div>
        <div className="modal-body">
          <label className="field">
            <span className="field-label">Name</span>
            <input
              autoFocus
              className="input"
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
              placeholder="Shaker pivot door, iron"
            />
          </label>
          <div className="row-2">
            <label className="field">
              <span className="field-label">Category</span>
              <select
                className="select"
                value={form.category}
                onChange={(e) => setForm({ ...form, category: e.target.value })}
              >
                <option value="iron">Iron door</option>
                <option value="wood">Wood door</option>
                <option value="barn">Barn door</option>
                <option value="pivot">Pivot door</option>
                <option value="patio">Patio door</option>
                <option value="fiberglass">Fiberglass door</option>
                <option value="interior">Interior door</option>
                <option value="architectural">Architectural</option>
                <option value="service">Service / install</option>
                <option value="other">Other</option>
              </select>
            </label>
            <label className="field">
              <span className="field-label">Unit</span>
              <select
                className="select"
                value={form.unit}
                onChange={(e) => setForm({ ...form, unit: e.target.value })}
              >
                <option value="each">Each</option>
                <option value="linear-ft">Linear ft</option>
                <option value="sq-ft">Sq ft</option>
              </select>
            </label>
          </div>
          <div className="row-2">
            <label className="field">
              <span className="field-label">Base price (USD)</span>
              <input
                className="input"
                type="number"
                min="0"
                step="0.01"
                value={form.basePrice}
                onChange={(e) => setForm({ ...form, basePrice: e.target.value })}
                placeholder="0.00"
              />
            </label>
            <label className="field">
              <span className="field-label">SKU</span>
              <input
                className="input"
                value={form.sku}
                onChange={(e) => setForm({ ...form, sku: e.target.value })}
                placeholder="Optional"
              />
            </label>
          </div>
          <label className="field">
            <span className="field-label">Lead time (days)</span>
            <input
              className="input"
              type="number"
              min="0"
              value={form.leadTimeDays}
              onChange={(e) =>
                setForm({ ...form, leadTimeDays: e.target.value })
              }
              placeholder="Optional"
            />
          </label>
          <label className="field">
            <span className="field-label">Description</span>
            <textarea
              className="textarea"
              value={form.description}
              onChange={(e) =>
                setForm({ ...form, description: e.target.value })
              }
            />
          </label>
          {err && <div className="error-text">{err}</div>}
        </div>
        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>Cancel</button>
          <button
            className="btn btn-primary"
            disabled={busy || !form.name.trim()}
            onClick={() => void save()}
          >
            {busy ? "Saving…" : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Inventory
// ---------------------------------------------------------------------------

function InventoryPage({
  org,
  currentUser,
}: {
  org: Organization;
  currentUser: User;
}) {
  const { data: materials } = db.useQuery<Material>("Material", {
    where: { orgId: org.id },
    orderBy: { name: "asc" },
  });
  const [addOpen, setAddOpen] = useState(false);
  const [adjust, setAdjust] = useState<Material | null>(null);

  return (
    <>
      <div className="page-header">
        <div>
          <div className="page-title">Shop floor</div>
          <div className="page-subtitle">
            Materials on hand. Adjust stock to log receipts, issues, and
            corrections.
          </div>
        </div>
        <button className="btn btn-primary" onClick={() => setAddOpen(true)}>
          <IconPlus />
          Add material
        </button>
      </div>
      {(materials ?? []).length === 0 ? (
        <div className="empty">
          <div className="empty-title">No materials tracked</div>
          <div className="empty-body">
            Add materials so you can log receipts and get reorder
            alerts.
          </div>
          <button className="btn btn-primary" onClick={() => setAddOpen(true)}>
            Add material
          </button>
        </div>
      ) : (
        <table className="table">
          <thead>
            <tr>
              <th>Name</th>
              <th>SKU</th>
              <th style={{ textAlign: "right" }}>On hand</th>
              <th style={{ textAlign: "right" }}>Reorder at</th>
              <th style={{ textAlign: "right" }}>Cost / unit</th>
              <th>Supplier</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {(materials ?? []).map((m) => {
              const low = m.stockQty <= m.reorderPoint;
              return (
                <tr key={m.id}>
                  <td style={{ fontWeight: 500 }}>{m.name}</td>
                  <td style={{ color: "var(--text-muted)" }}>{m.sku || "—"}</td>
                  <td
                    style={{
                      textAlign: "right",
                      fontVariantNumeric: "tabular-nums",
                    }}
                  >
                    <span className={"pill " + (low ? "pill-warning" : "pill-gray")}>
                      {m.stockQty} {m.unit}
                    </span>
                  </td>
                  <td
                    style={{
                      textAlign: "right",
                      color: "var(--text-muted)",
                      fontVariantNumeric: "tabular-nums",
                    }}
                  >
                    {m.reorderPoint} {m.unit}
                  </td>
                  <td
                    style={{
                      textAlign: "right",
                      fontVariantNumeric: "tabular-nums",
                    }}
                  >
                    {money(m.costPerUnit)}
                  </td>
                  <td style={{ color: "var(--text-muted)" }}>
                    {m.supplier || "—"}
                  </td>
                  <td style={{ textAlign: "right" }}>
                    <button
                      className="btn btn-ghost"
                      style={{ padding: "4px 10px" }}
                      onClick={() => setAdjust(m)}
                    >
                      Adjust
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
      {addOpen && <AddMaterialModal onClose={() => setAddOpen(false)} />}
      {adjust && (
        <AdjustStockModal
          material={adjust}
          onClose={() => setAdjust(null)}
        />
      )}
    </>
  );
}

function AddMaterialModal({ onClose }: { onClose: () => void }) {
  const [form, setForm] = useState({
    name: "",
    unit: "each",
    costPerUnit: "",
    reorderPoint: "",
    initialStock: "",
    sku: "",
    supplier: "",
  });
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function save() {
    setBusy(true);
    setErr(null);
    try {
      await callFn("createMaterial", {
        name: form.name,
        unit: form.unit,
        costPerUnit: parseFloat(form.costPerUnit) || 0,
        reorderPoint: form.reorderPoint ? parseFloat(form.reorderPoint) : 0,
        initialStock: form.initialStock ? parseFloat(form.initialStock) : 0,
        sku: form.sku || undefined,
        supplier: form.supplier || undefined,
      });
      onClose();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">Add material</div>
        <div className="modal-subtitle">Lumber, iron stock, glass, finishes — anything you stock.</div>
        <div className="modal-body">
          <label className="field">
            <span className="field-label">Name</span>
            <input
              autoFocus
              className="input"
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
              placeholder="Red oak 4/4"
            />
          </label>
          <div className="row-2">
            <label className="field">
              <span className="field-label">Unit</span>
              <select
                className="select"
                value={form.unit}
                onChange={(e) => setForm({ ...form, unit: e.target.value })}
              >
                <option value="each">Each</option>
                <option value="board-ft">Board ft</option>
                <option value="ft">Linear ft</option>
                <option value="sq-ft">Sq ft</option>
                <option value="lb">Pound</option>
              </select>
            </label>
            <label className="field">
              <span className="field-label">Cost / unit (USD)</span>
              <input
                className="input"
                type="number"
                step="0.01"
                min="0"
                value={form.costPerUnit}
                onChange={(e) =>
                  setForm({ ...form, costPerUnit: e.target.value })
                }
              />
            </label>
          </div>
          <div className="row-2">
            <label className="field">
              <span className="field-label">Initial stock</span>
              <input
                className="input"
                type="number"
                step="0.1"
                min="0"
                value={form.initialStock}
                onChange={(e) =>
                  setForm({ ...form, initialStock: e.target.value })
                }
              />
            </label>
            <label className="field">
              <span className="field-label">Reorder point</span>
              <input
                className="input"
                type="number"
                step="0.1"
                min="0"
                value={form.reorderPoint}
                onChange={(e) =>
                  setForm({ ...form, reorderPoint: e.target.value })
                }
              />
            </label>
          </div>
          <div className="row-2">
            <label className="field">
              <span className="field-label">SKU</span>
              <input
                className="input"
                value={form.sku}
                onChange={(e) => setForm({ ...form, sku: e.target.value })}
              />
            </label>
            <label className="field">
              <span className="field-label">Supplier</span>
              <input
                className="input"
                value={form.supplier}
                onChange={(e) => setForm({ ...form, supplier: e.target.value })}
              />
            </label>
          </div>
          {err && <div className="error-text">{err}</div>}
        </div>
        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>Cancel</button>
          <button
            className="btn btn-primary"
            disabled={busy || !form.name.trim()}
            onClick={() => void save()}
          >
            {busy ? "Saving…" : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
}

function AdjustStockModal({
  material,
  onClose,
}: {
  material: Material;
  onClose: () => void;
}) {
  const [delta, setDelta] = useState("");
  const [reason, setReason] = useState<"receipt" | "issue" | "adjust" | "waste">(
    "receipt",
  );
  const [reference, setReference] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function save() {
    setBusy(true);
    setErr(null);
    try {
      const signed = parseFloat(delta);
      // Issue and waste should subtract from stock even if the user entered
      // a positive number — flip the sign for their convenience.
      const adjustedDelta =
        reason === "issue" || reason === "waste"
          ? -Math.abs(signed)
          : reason === "adjust"
            ? signed // adjust keeps the sign as entered
            : Math.abs(signed); // receipts are always positive
      await callFn("adjustStock", {
        materialId: material.id,
        delta: adjustedDelta,
        reason,
        reference: reference || undefined,
      });
      onClose();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">Adjust stock</div>
        <div className="modal-subtitle">
          {material.name} — currently {material.stockQty} {material.unit}
        </div>
        <div className="modal-body">
          <div className="row-2">
            <label className="field">
              <span className="field-label">Reason</span>
              <select
                className="select"
                value={reason}
                onChange={(e) => setReason(e.target.value as typeof reason)}
              >
                <option value="receipt">Receipt (+)</option>
                <option value="issue">Issue to order (−)</option>
                <option value="adjust">Manual adjust (±)</option>
                <option value="waste">Waste / loss (−)</option>
              </select>
            </label>
            <label className="field">
              <span className="field-label">
                Quantity ({material.unit})
                {reason === "adjust" && (
                  <span style={{ color: "var(--text-dim)", marginLeft: 4 }}>
                    signed
                  </span>
                )}
              </span>
              <input
                className="input"
                type="number"
                step="0.1"
                value={delta}
                onChange={(e) => setDelta(e.target.value)}
                autoFocus
              />
            </label>
          </div>
          <label className="field">
            <span className="field-label">Reference</span>
            <input
              className="input"
              value={reference}
              onChange={(e) => setReference(e.target.value)}
              placeholder="PO number, order #, etc."
            />
          </label>
          {err && <div className="error-text">{err}</div>}
        </div>
        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>Cancel</button>
          <button
            className="btn btn-primary"
            disabled={busy || !delta}
            onClick={() => void save()}
          >
            {busy ? "Saving…" : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Orders
// ---------------------------------------------------------------------------

const ORDER_FILTER_FIELDS: FilterField[] = [
  { key: "number", label: "Number", kind: "string" },
  {
    key: "status",
    label: "Status",
    kind: "enum",
    options: [
      { value: "confirmed", label: "Designed" },
      { value: "in_production", label: "In fabrication" },
      { value: "ready", label: "Finishing" },
      { value: "shipped", label: "Ready to install" },
      { value: "delivered", label: "Installed" },
      { value: "cancelled", label: "Cancelled" },
    ],
  },
  { key: "total", label: "Total", kind: "number" },
  { key: "subtotal", label: "Subtotal", kind: "number" },
  { key: "createdAt", label: "Created", kind: "date" },
  { key: "dueDate", label: "Due date", kind: "date" },
];

function OrdersPage({ org }: { org: Organization }) {
  const [filter, setFilter] = useState<FilterState>(EMPTY_FILTER);
  const { data: orders } = db.useQuery<Order>("Order", {
    where: {
      orgId: org.id,
      ...buildQueryFilter(ORDER_FILTER_FIELDS, filter),
    },
    orderBy: { createdAt: "desc" },
  });
  const filtered = useMemo(
    () => applyFilterClient(orders ?? [], ORDER_FILTER_FIELDS, filter),
    [orders, filter],
  );
  const [addOpen, setAddOpen] = useState(false);

  return (
    <>
      <div className="page-header">
        <div>
          <div className="page-title">Projects</div>
          <div className="page-subtitle">
            Track jobs from design through install.
          </div>
        </div>
        <button className="btn btn-primary" onClick={() => setAddOpen(true)}>
          <IconPlus />
          New project
        </button>
      </div>
      <FilterBar
        fields={ORDER_FILTER_FIELDS}
        value={filter}
        onChange={setFilter}
      />
      {filtered.length === 0 ? (
        <div className="empty">
          <div className="empty-title">
            {(orders ?? []).length === 0
              ? "No projects yet"
              : "No projects match these filters"}
          </div>
          <div className="empty-body">
            {(orders ?? []).length === 0
              ? "Book a project once a consultation is signed."
              : "Adjust the filters above to see more."}
          </div>
          <button className="btn btn-primary" onClick={() => setAddOpen(true)}>
            New project
          </button>
        </div>
      ) : (
        <table className="table">
          <thead>
            <tr>
              <th>Number</th>
              <th>Customer</th>
              <th>Status</th>
              <th style={{ textAlign: "right" }}>Total</th>
              <th>Due</th>
              <th>Created</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {filtered.map((o) => (
              <OrderRow key={o.id} order={o} />
            ))}
          </tbody>
        </table>
      )}
      {addOpen && <NewOrderModal org={org} onClose={() => setAddOpen(false)} />}
    </>
  );
}

function OrderRow({ order }: { order: Order }) {
  const { data: customer } = db.useQueryOne<Customer>(
    "Customer",
    order.customerId,
  );
  const [busy, setBusy] = useState(false);
  const style = ORDER_STATUS_STYLE[order.status] ?? {
    label: order.status,
    className: "pill-gray",
  };

  const next = (() => {
    const order_ = [
      "confirmed",
      "in_production",
      "ready",
      "shipped",
      "delivered",
    ];
    const i = order_.indexOf(order.status);
    if (i < 0 || i >= order_.length - 1) return null;
    return order_[i + 1];
  })();

  async function advance() {
    if (!next) return;
    setBusy(true);
    try {
      await callFn("advanceOrderStatus", {
        orderId: order.id,
        status: next,
      });
    } catch (e) {
      alert((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <tr>
      <td style={{ fontWeight: 500, fontVariantNumeric: "tabular-nums" }}>
        {order.number}
      </td>
      <td>{customer?.name ?? "…"}</td>
      <td>
        <span className={"pill " + style.className}>
          <span className="pill-dot" />
          {style.label}
        </span>
      </td>
      <td
        style={{ textAlign: "right", fontVariantNumeric: "tabular-nums" }}
      >
        {money(order.total)}
      </td>
      <td style={{ color: "var(--text-muted)" }}>
        {order.dueDate ? formatDate(order.dueDate) : "—"}
      </td>
      <td style={{ color: "var(--text-muted)" }}>
        {formatDate(order.createdAt)}
      </td>
      <td style={{ textAlign: "right" }}>
        {next && order.status !== "cancelled" && (
          <button
            className="btn btn-ghost"
            style={{ padding: "4px 10px" }}
            onClick={() => void advance()}
            disabled={busy}
          >
            Mark {ORDER_STATUS_STYLE[next]?.label || next}
          </button>
        )}
      </td>
    </tr>
  );
}

type LineDraft = {
  productId: string;
  description: string;
  qty: string;
  unitPrice: string;
};

function NewOrderModal({
  org,
  onClose,
}: {
  org: Organization;
  onClose: () => void;
}) {
  const { data: customers } = db.useQuery<Customer>("Customer", {
    where: { orgId: org.id },
  });
  const { data: products } = db.useQuery<Product>("Product", {
    where: { orgId: org.id },
  });

  const [customerId, setCustomerId] = useState("");
  const [dueDate, setDueDate] = useState("");
  const [notes, setNotes] = useState("");
  const [lines, setLines] = useState<LineDraft[]>([
    { productId: "", description: "", qty: "1", unitPrice: "" },
  ]);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const subtotal = lines.reduce((sum, l) => {
    const q = parseFloat(l.qty) || 0;
    const p = parseFloat(l.unitPrice) || 0;
    return sum + q * p;
  }, 0);

  function updateLine(i: number, patch: Partial<LineDraft>) {
    setLines((prev) => prev.map((l, idx) => (idx === i ? { ...l, ...patch } : l)));
  }

  function pickProduct(i: number, productId: string) {
    const prod = (products ?? []).find((p) => p.id === productId);
    if (!prod) return;
    updateLine(i, {
      productId,
      description: prod.name,
      unitPrice: prod.basePrice.toString(),
    });
  }

  async function save() {
    setBusy(true);
    setErr(null);
    try {
      if (!customerId) throw new Error("pick a customer");
      const prepared = lines
        .filter((l) => l.productId)
        .map((l) => ({
          productId: l.productId,
          description: l.description,
          qty: parseFloat(l.qty) || 0,
          unitPrice: parseFloat(l.unitPrice) || 0,
        }));
      if (prepared.length === 0) throw new Error("add at least one line");
      await callFn("createOrder", {
        customerId,
        dueDate: dueDate || undefined,
        notes: notes || undefined,
        lines: prepared,
      });
      onClose();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal"
        onClick={(e) => e.stopPropagation()}
        style={{ width: 640 }}
      >
        <div className="modal-title">New project</div>
        <div className="modal-subtitle">Confirm the project — CAD approved, ready to fabricate.</div>
        <div className="modal-body">
          <div className="row-2">
            <label className="field">
              <span className="field-label">Customer</span>
              <select
                className="select"
                value={customerId}
                onChange={(e) => setCustomerId(e.target.value)}
              >
                <option value="">Choose a customer…</option>
                {(customers ?? []).map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.name}
                    {c.company ? ` · ${c.company}` : ""}
                  </option>
                ))}
              </select>
            </label>
            <label className="field">
              <span className="field-label">Due date</span>
              <input
                className="input"
                type="date"
                value={dueDate}
                onChange={(e) => setDueDate(e.target.value)}
              />
            </label>
          </div>

          <div
            style={{
              fontSize: 11,
              fontWeight: 600,
              letterSpacing: "0.06em",
              textTransform: "uppercase",
              color: "var(--text-dim)",
              marginTop: 8,
              marginBottom: 8,
            }}
          >
            Line items
          </div>
          <table
            className="table"
            style={{ border: "1px solid var(--border)", marginBottom: 8 }}
          >
            <thead>
              <tr>
                <th>Product</th>
                <th>Description</th>
                <th style={{ width: 80, textAlign: "right" }}>Qty</th>
                <th style={{ width: 110, textAlign: "right" }}>Price</th>
                <th style={{ width: 100, textAlign: "right" }}>Total</th>
                <th style={{ width: 32 }} />
              </tr>
            </thead>
            <tbody>
              {lines.map((l, i) => {
                const lineTotal =
                  (parseFloat(l.qty) || 0) * (parseFloat(l.unitPrice) || 0);
                return (
                  <tr key={i}>
                    <td>
                      <select
                        className="select"
                        value={l.productId}
                        onChange={(e) => pickProduct(i, e.target.value)}
                      >
                        <option value="">—</option>
                        {(products ?? []).map((p) => (
                          <option key={p.id} value={p.id}>
                            {p.name}
                          </option>
                        ))}
                      </select>
                    </td>
                    <td>
                      <input
                        className="input"
                        value={l.description}
                        onChange={(e) =>
                          updateLine(i, { description: e.target.value })
                        }
                      />
                    </td>
                    <td>
                      <input
                        className="input"
                        type="number"
                        min="0"
                        step="1"
                        style={{ textAlign: "right" }}
                        value={l.qty}
                        onChange={(e) => updateLine(i, { qty: e.target.value })}
                      />
                    </td>
                    <td>
                      <input
                        className="input"
                        type="number"
                        min="0"
                        step="0.01"
                        style={{ textAlign: "right" }}
                        value={l.unitPrice}
                        onChange={(e) =>
                          updateLine(i, { unitPrice: e.target.value })
                        }
                      />
                    </td>
                    <td
                      style={{
                        textAlign: "right",
                        fontVariantNumeric: "tabular-nums",
                      }}
                    >
                      {money(lineTotal)}
                    </td>
                    <td>
                      <button
                        className="btn btn-ghost"
                        style={{ padding: "4px 6px" }}
                        onClick={() =>
                          setLines((prev) => prev.filter((_, idx) => idx !== i))
                        }
                        disabled={lines.length === 1}
                        aria-label="Remove line"
                        title="Remove"
                      >
                        ×
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          <button
            className="btn btn-ghost"
            onClick={() =>
              setLines((prev) => [
                ...prev,
                { productId: "", description: "", qty: "1", unitPrice: "" },
              ])
            }
          >
            <IconPlus /> Add line
          </button>
          <div style={{ textAlign: "right", marginTop: 12, fontSize: 14 }}>
            <span style={{ color: "var(--text-muted)" }}>Subtotal: </span>
            <strong style={{ fontVariantNumeric: "tabular-nums" }}>
              {money(subtotal)}
            </strong>
          </div>
          <label className="field" style={{ marginTop: 12 }}>
            <span className="field-label">Notes</span>
            <textarea
              className="textarea"
              value={notes}
              onChange={(e) => setNotes(e.target.value)}
            />
          </label>
          {err && <div className="error-text">{err}</div>}
        </div>
        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>
            Cancel
          </button>
          <button
            className="btn btn-primary"
            disabled={busy}
            onClick={() => void save()}
          >
            {busy ? "Saving…" : "Create project"}
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Team
// ---------------------------------------------------------------------------

function TeamPage({
  org,
  currentUser,
  myRole,
}: {
  org: Organization;
  currentUser: User;
  myRole: string;
}) {
  const { data: members } = db.useQuery<OrgMember>("OrgMember", {
    where: { orgId: org.id },
  });
  const { data: invites } = db.useQuery<OrgInvite>("OrgInvite", {
    where: { orgId: org.id },
  });
  const [inviteOpen, setInviteOpen] = useState(false);
  const canInvite = myRole === "owner" || myRole === "admin";
  const pendingInvites = (invites ?? []).filter((i) => !i.acceptedAt);

  return (
    <>
      <div className="page-header">
        <div>
          <div className="page-title">Team</div>
          <div className="page-subtitle">
            Who has access and what they can do.
          </div>
        </div>
        {canInvite && (
          <button className="btn btn-primary" onClick={() => setInviteOpen(true)}>
            <IconPlus />
            Invite member
          </button>
        )}
      </div>
      <table className="table">
        <thead>
          <tr>
            <th>Name</th>
            <th>Email</th>
            <th>Role</th>
            <th>Joined</th>
          </tr>
        </thead>
        <tbody>
          {(members ?? []).map((m) => (
            <MemberRow
              key={m.id}
              member={m}
              canManage={canInvite}
              myUserId={currentUser.id}
            />
          ))}
          {pendingInvites.map((inv) => (
            <PendingInviteRow key={inv.id} invite={inv} />
          ))}
        </tbody>
      </table>
      {inviteOpen && <InviteMemberModal onClose={() => setInviteOpen(false)} />}
    </>
  );
}

function MemberRow({
  member,
  canManage,
  myUserId,
}: {
  member: OrgMember;
  canManage: boolean;
  myUserId: string;
}) {
  const { data: user } = db.useQueryOne<User>("User", member.userId);
  const isMe = member.userId === myUserId;
  const update = db.useMutation<
    { memberId: string; role: string },
    unknown
  >("updateMemberRole");

  async function changeRole(role: string) {
    try {
      await update.mutate({ memberId: member.id, role });
    } catch (e) {
      alert((e as Error).message);
    }
  }

  return (
    <tr>
      <td>
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <div
            className="avatar avatar-sm"
            style={{ backgroundColor: user?.avatarColor || "#c7d2fe" }}
          >
            {initials(user?.displayName)}
          </div>
          <div>
            <div style={{ fontWeight: 500 }}>{user?.displayName ?? "…"}</div>
            {isMe && (
              <div style={{ fontSize: 11, color: "var(--text-dim)" }}>you</div>
            )}
          </div>
        </div>
      </td>
      <td style={{ color: "var(--text-muted)" }}>{user?.email ?? "…"}</td>
      <td>
        {canManage && !isMe ? (
          <select
            className="select"
            style={{ width: "auto", padding: "4px 8px" }}
            value={member.role}
            onChange={(e) => void changeRole(e.target.value)}
          >
            <option value="owner">owner</option>
            <option value="admin">admin</option>
            <option value="estimator">estimator</option>
            <option value="production">production</option>
            <option value="viewer">viewer</option>
          </select>
        ) : (
          <span className="pill pill-gray">{member.role}</span>
        )}
      </td>
      <td style={{ color: "var(--text-muted)" }}>{formatDate(member.joinedAt)}</td>
    </tr>
  );
}

function PendingInviteRow({ invite }: { invite: OrgInvite }) {
  return (
    <tr>
      <td>
        <span className="pill pill-warning">Invite pending</span>
      </td>
      <td style={{ color: "var(--text-muted)" }}>{invite.email}</td>
      <td>
        <span className="pill pill-gray">{invite.role}</span>
      </td>
      <td style={{ color: "var(--text-muted)" }}>
        Sent {formatDate(invite.createdAt)}
      </td>
    </tr>
  );
}

function InviteMemberModal({ onClose }: { onClose: () => void }) {
  const [email, setEmail] = useState("");
  const [role, setRole] = useState("estimator");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function send() {
    setBusy(true);
    setErr(null);
    try {
      await callFn("inviteMember", { email, role });
      onClose();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">Invite member</div>
        <div className="modal-subtitle">
          They'll see the invite on next login with that email.
        </div>
        <div className="modal-body">
          <label className="field">
            <span className="field-label">Email</span>
            <input
              autoFocus
              className="input"
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="coworker@shop.com"
            />
          </label>
          <label className="field">
            <span className="field-label">Role</span>
            <select
              className="select"
              value={role}
              onChange={(e) => setRole(e.target.value)}
            >
              <option value="admin">Admin — everything except delete org</option>
              <option value="estimator">Estimator — quotes + orders + catalog</option>
              <option value="production">Production — stock + order status</option>
              <option value="viewer">Viewer — read-only</option>
            </select>
          </label>
          {err && <div className="error-text">{err}</div>}
        </div>
        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>Cancel</button>
          <button
            className="btn btn-primary"
            disabled={busy || !email.trim()}
            onClick={() => void send()}
          >
            {busy ? "Sending…" : "Send invite"}
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Create-org modal
// ---------------------------------------------------------------------------

function CreateOrgModal({
  onClose,
  onCreated,
}: {
  onClose: () => void;
  onCreated: (orgId: string) => void;
}) {
  const [name, setName] = useState("");
  const [slug, setSlug] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  // Auto-derive slug from name until user types one in manually.
  const [slugEdited, setSlugEdited] = useState(false);
  useEffect(() => {
    if (slugEdited) return;
    setSlug(
      name
        .toLowerCase()
        .trim()
        .replace(/[^a-z0-9\s-]/g, "")
        .replace(/\s+/g, "-")
        .slice(0, 50),
    );
  }, [name, slugEdited]);

  async function save() {
    setBusy(true);
    setErr(null);
    try {
      const res = await callFn<{ orgId: string }>("createOrganization", {
        name,
        slug,
      });
      onCreated(res.orgId);
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">Create your shop workspace</div>
        <div className="modal-subtitle">
          One workspace per shop. Holds your customers, catalog, and projects.
        </div>
        <div className="modal-body">
          <label className="field">
            <span className="field-label">Company name</span>
            <input
              autoFocus
              className="input"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Dallas Door Designs"
            />
          </label>
          <label className="field">
            <span className="field-label">URL slug</span>
            <input
              className="input"
              value={slug}
              onChange={(e) => {
                setSlug(e.target.value.toLowerCase());
                setSlugEdited(true);
              }}
              placeholder="dallas-door-designs"
            />
          </label>
          {err && <div className="error-text">{err}</div>}
        </div>
        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>Cancel</button>
          <button
            className="btn btn-primary"
            disabled={busy || !name.trim() || !slug.trim()}
            onClick={() => void save()}
          >
            {busy ? "Creating…" : "Create workspace"}
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Analytics — user-saved dashboard panels backed by POST /api/aggregate
// ---------------------------------------------------------------------------

function AnalyticsPage({ org }: { org: Organization }) {
  const { data: panels } = db.useQuery<DashboardPanel>("DashboardPanel", {
    where: { orgId: org.id },
    orderBy: { sortOrder: "asc" },
  });
  const [builderOpen, setBuilderOpen] = useState(false);

  return (
    <>
      <div className="page-header">
        <div>
          <div className="page-title">Analytics</div>
          <div className="page-subtitle">
            Live metrics — panels refresh automatically as projects, customers,
            and inventory change.
          </div>
        </div>
        <button className="btn btn-primary" onClick={() => setBuilderOpen(true)}>
          <IconPlus />
          Add panel
        </button>
      </div>
      {(panels ?? []).length === 0 ? (
        <div className="empty">
          <div className="empty-title">No panels yet</div>
          <div className="empty-body">
            Build a chart from any table — totals, groupings, time-series.
          </div>
          <button className="btn btn-primary" onClick={() => setBuilderOpen(true)}>
            Add panel
          </button>
        </div>
      ) : (
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "repeat(auto-fill, minmax(320px, 1fr))",
            gap: 14,
          }}
        >
          {(panels ?? []).map((p) => (
            <PanelCard key={p.id} panel={p} />
          ))}
        </div>
      )}
      {builderOpen && (
        <PanelBuilder onClose={() => setBuilderOpen(false)} />
      )}
    </>
  );
}

function PanelCard({ panel }: { panel: DashboardPanel }) {
  // Parse once per render of this panel — spec is just a JSON string.
  const spec = useMemo<AggregateSpec>(() => {
    try {
      return JSON.parse(panel.specJson) as AggregateSpec;
    } catch {
      return {};
    }
  }, [panel.specJson]);

  const { data, loading, error } = db.useAggregate<Record<string, unknown>>(
    panel.entity,
    spec,
  );

  async function remove() {
    if (!confirm(`Remove panel "${panel.title}"?`)) return;
    try {
      await callFn("deletePanel", { panelId: panel.id });
    } catch (e) {
      alert((e as Error).message);
    }
  }

  return (
    <div className="card" style={{ padding: 0, overflow: "hidden" }}>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          padding: "12px 16px",
          borderBottom: "1px solid var(--border)",
        }}
      >
        <div>
          <div style={{ fontSize: 13.5, fontWeight: 600 }}>{panel.title}</div>
          <div style={{ fontSize: 11, color: "var(--text-dim)" }}>
            {panel.entity} · {panel.chartKind}
          </div>
        </div>
        <button
          className="btn btn-ghost"
          onClick={() => void remove()}
          title="Remove panel"
          style={{ padding: "4px 8px" }}
        >
          ×
        </button>
      </div>
      <div style={{ padding: 16, minHeight: 160 }}>
        {error ? (
          <div className="error-text">{error.message}</div>
        ) : loading && !data ? (
          <div style={{ color: "var(--text-dim)", fontSize: 12 }}>Loading…</div>
        ) : (
          <ChartRenderer kind={panel.chartKind} data={data ?? []} spec={spec} />
        )}
      </div>
    </div>
  );
}

/**
 * Zero-dependency SVG chart renderer. Each kind handles a different output
 * shape from the aggregate endpoint:
 * - "number": single row, picks the first numeric value.
 * - "bar":    groupBy axis + one metric column.
 * - "line":   groupBy axis (usually a date bucket) + one metric column.
 */
function ChartRenderer({
  kind,
  data,
  spec,
}: {
  kind: string;
  data: Record<string, unknown>[];
  spec: AggregateSpec;
}) {
  if (data.length === 0) {
    return (
      <div style={{ color: "var(--text-dim)", fontSize: 12 }}>
        No rows match this query.
      </div>
    );
  }

  // Figure out which keys are group dimensions vs metrics based on the spec.
  const groupKeys = useMemo(() => {
    return (spec.groupBy ?? []).map((g) =>
      typeof g === "string" ? g : `${g.field}_${g.bucket}`,
    );
  }, [spec]);
  const metricKeys = useMemo(() => {
    const keys = new Set(Object.keys(data[0] ?? {}));
    for (const g of groupKeys) keys.delete(g);
    return Array.from(keys);
  }, [data, groupKeys]);

  if (kind === "number") {
    const metric = metricKeys[0] ?? "count";
    const val = Number(data[0]?.[metric] ?? 0);
    const display = Number.isFinite(val)
      ? val.toLocaleString(undefined, { maximumFractionDigits: 2 })
      : String(val);
    return (
      <div>
        <div
          style={{
            fontSize: 34,
            fontWeight: 700,
            letterSpacing: "-0.02em",
            fontVariantNumeric: "tabular-nums",
          }}
        >
          {display}
        </div>
        <div style={{ fontSize: 11.5, color: "var(--text-dim)", marginTop: 2 }}>
          {metric}
        </div>
      </div>
    );
  }

  const labelKey = groupKeys[0] ?? Object.keys(data[0] ?? {})[0] ?? "";
  const metric = metricKeys[0] ?? "count";
  const rows = data.map((r) => ({
    label: String(r[labelKey] ?? "—"),
    value: Number(r[metric] ?? 0),
  }));
  const max = Math.max(1, ...rows.map((r) => r.value));

  if (kind === "line") {
    const W = 280;
    const H = 120;
    const pad = { top: 10, right: 4, bottom: 18, left: 4 };
    const innerW = W - pad.left - pad.right;
    const innerH = H - pad.top - pad.bottom;
    const step = rows.length > 1 ? innerW / (rows.length - 1) : 0;
    const points = rows.map((r, i) => {
      const x = pad.left + i * step;
      const y = pad.top + innerH - (r.value / max) * innerH;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    });
    const area = `M${points[0]} L${points.join(" L")} L${pad.left + innerW},${
      pad.top + innerH
    } L${pad.left},${pad.top + innerH} Z`;
    return (
      <svg viewBox={`0 0 ${W} ${H}`} width="100%" style={{ display: "block" }}>
        <path d={area} fill="var(--accent-soft)" />
        <polyline
          points={points.join(" ")}
          fill="none"
          stroke="var(--accent)"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
        {rows.map((r, i) => {
          const x = pad.left + i * step;
          const y = pad.top + innerH - (r.value / max) * innerH;
          return (
            <circle key={i} cx={x} cy={y} r="2.5" fill="var(--accent)" />
          );
        })}
        {/* x-axis labels — first, middle, last */}
        {[0, Math.floor(rows.length / 2), rows.length - 1].map((i) =>
          rows[i] ? (
            <text
              key={i}
              x={pad.left + i * step}
              y={H - 4}
              fontSize="9"
              textAnchor={i === 0 ? "start" : i === rows.length - 1 ? "end" : "middle"}
              fill="var(--text-dim)"
            >
              {rows[i].label}
            </text>
          ) : null,
        )}
      </svg>
    );
  }

  // Default: bar chart
  const shown = rows.slice(0, 12);
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      {shown.map((r, i) => {
        const pct = Math.round((r.value / max) * 100);
        return (
          <div key={i} style={{ display: "flex", alignItems: "center", gap: 10 }}>
            <div
              style={{
                flex: "0 0 90px",
                fontSize: 11.5,
                color: "var(--text-muted)",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
              title={r.label}
            >
              {r.label}
            </div>
            <div
              style={{
                flex: 1,
                background: "var(--surface-hover)",
                borderRadius: 4,
                height: 16,
                position: "relative",
                overflow: "hidden",
              }}
            >
              <div
                style={{
                  width: `${pct}%`,
                  height: "100%",
                  background: "var(--accent)",
                  borderRadius: 4,
                  transition: "width 300ms ease",
                }}
              />
            </div>
            <div
              style={{
                flex: "0 0 52px",
                fontSize: 12,
                fontVariantNumeric: "tabular-nums",
                textAlign: "right",
                fontWeight: 500,
              }}
            >
              {r.value.toLocaleString(undefined, { maximumFractionDigits: 2 })}
            </div>
          </div>
        );
      })}
    </div>
  );
}

/**
 * Builder modal — pick an entity + chart kind + metric + group-by + filter.
 * Keeps the schema knowledge in the demo so there's no framework dependency
 * on introspecting the manifest at runtime.
 */
const ANALYTICS_ENTITIES: {
  name: string;
  label: string;
  fields: { key: string; kind: "number" | "string" | "date"; label: string }[];
}[] = [
  {
    name: "Order",
    label: "Projects",
    fields: [
      { key: "status", kind: "string", label: "Status" },
      { key: "total", kind: "number", label: "Total" },
      { key: "subtotal", kind: "number", label: "Subtotal" },
      { key: "createdAt", kind: "date", label: "Created" },
      { key: "customerId", kind: "string", label: "Customer" },
    ],
  },
  {
    name: "Customer",
    label: "Customers",
    fields: [
      { key: "createdAt", kind: "date", label: "Created" },
      { key: "city", kind: "string", label: "City" },
      { key: "state", kind: "string", label: "State" },
    ],
  },
  {
    name: "Product",
    label: "Catalog",
    fields: [
      { key: "category", kind: "string", label: "Category" },
      { key: "basePrice", kind: "number", label: "Base price" },
      { key: "createdAt", kind: "date", label: "Created" },
    ],
  },
  {
    name: "Material",
    label: "Materials",
    fields: [
      { key: "stockQty", kind: "number", label: "Stock qty" },
      { key: "reorderPoint", kind: "number", label: "Reorder point" },
      { key: "costPerUnit", kind: "number", label: "Cost / unit" },
    ],
  },
];

function PanelBuilder({ onClose }: { onClose: () => void }) {
  const [title, setTitle] = useState("");
  const [entityName, setEntityName] = useState("Order");
  const [chartKind, setChartKind] = useState<"number" | "bar" | "line">(
    "number",
  );
  const [metric, setMetric] = useState("count:*");
  const [groupField, setGroupField] = useState<string>("");
  const [bucket, setBucket] = useState<"hour" | "day" | "week" | "month" | "year">(
    "day",
  );
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const entityDef = ANALYTICS_ENTITIES.find((e) => e.name === entityName)!;
  const numericFields = entityDef.fields.filter((f) => f.kind === "number");
  const groupableFields = entityDef.fields.filter((f) => f.kind !== "number");

  // Line chart needs a time axis; default groupField to the first date when
  // switching to line so the user doesn't have to.
  useEffect(() => {
    if (chartKind === "line") {
      const firstDate = entityDef.fields.find((f) => f.kind === "date");
      if (firstDate) setGroupField(firstDate.key);
    } else if (chartKind === "number") {
      setGroupField("");
    }
  }, [chartKind, entityName]);

  const spec: AggregateSpec = useMemo(() => {
    const s: AggregateSpec = {};
    if (metric.startsWith("count:")) {
      s.count = metric.slice("count:".length);
    } else {
      const [fn, field] = metric.split(":");
      if (fn === "sum") s.sum = [field];
      else if (fn === "avg") s.avg = [field];
      else if (fn === "min") s.min = [field];
      else if (fn === "max") s.max = [field];
    }
    if (chartKind !== "number" && groupField) {
      const fieldDef = entityDef.fields.find((f) => f.key === groupField);
      if (fieldDef?.kind === "date") {
        s.groupBy = [{ field: groupField, bucket }];
      } else {
        s.groupBy = [groupField];
      }
    }
    return s;
  }, [metric, chartKind, groupField, bucket, entityDef]);

  async function save() {
    setBusy(true);
    setErr(null);
    try {
      if (!title.trim()) throw new Error("title is required");
      await callFn("createPanel", {
        title: title.trim(),
        entity: entityName,
        chartKind,
        specJson: JSON.stringify(spec),
      });
      onClose();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal"
        onClick={(e) => e.stopPropagation()}
        style={{ width: 560 }}
      >
        <div className="modal-title">New panel</div>
        <div className="modal-subtitle">
          Pick a table, a metric, and how to slice it. Preview updates live.
        </div>
        <div className="modal-body">
          <label className="field">
            <span className="field-label">Panel title</span>
            <input
              autoFocus
              className="input"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="Revenue this month"
            />
          </label>
          <div className="row-2">
            <label className="field">
              <span className="field-label">Table</span>
              <select
                className="select"
                value={entityName}
                onChange={(e) => {
                  setEntityName(e.target.value);
                  setMetric("count:*");
                  setGroupField("");
                }}
              >
                {ANALYTICS_ENTITIES.map((e) => (
                  <option key={e.name} value={e.name}>
                    {e.label}
                  </option>
                ))}
              </select>
            </label>
            <label className="field">
              <span className="field-label">Chart</span>
              <select
                className="select"
                value={chartKind}
                onChange={(e) =>
                  setChartKind(e.target.value as typeof chartKind)
                }
              >
                <option value="number">Number</option>
                <option value="bar">Bar</option>
                <option value="line">Line (over time)</option>
              </select>
            </label>
          </div>
          <label className="field">
            <span className="field-label">Metric</span>
            <select
              className="select"
              value={metric}
              onChange={(e) => setMetric(e.target.value)}
            >
              <option value="count:*">Count of rows</option>
              {numericFields.flatMap((f) => [
                <option key={`sum:${f.key}`} value={`sum:${f.key}`}>
                  Sum of {f.label}
                </option>,
                <option key={`avg:${f.key}`} value={`avg:${f.key}`}>
                  Average of {f.label}
                </option>,
                <option key={`min:${f.key}`} value={`min:${f.key}`}>
                  Min of {f.label}
                </option>,
                <option key={`max:${f.key}`} value={`max:${f.key}`}>
                  Max of {f.label}
                </option>,
              ])}
            </select>
          </label>
          {chartKind !== "number" && (
            <div className="row-2">
              <label className="field">
                <span className="field-label">Group by</span>
                <select
                  className="select"
                  value={groupField}
                  onChange={(e) => setGroupField(e.target.value)}
                >
                  <option value="">—</option>
                  {(chartKind === "line"
                    ? entityDef.fields.filter((f) => f.kind === "date")
                    : groupableFields
                  ).map((f) => (
                    <option key={f.key} value={f.key}>
                      {f.label}
                    </option>
                  ))}
                </select>
              </label>
              {chartKind === "line" && (
                <label className="field">
                  <span className="field-label">Bucket</span>
                  <select
                    className="select"
                    value={bucket}
                    onChange={(e) =>
                      setBucket(e.target.value as typeof bucket)
                    }
                  >
                    <option value="hour">Hour</option>
                    <option value="day">Day</option>
                    <option value="week">Week</option>
                    <option value="month">Month</option>
                    <option value="year">Year</option>
                  </select>
                </label>
              )}
            </div>
          )}

          {/* Live preview — uses useAggregate so the chart updates as they tweak. */}
          <div
            style={{
              marginTop: 12,
              padding: 14,
              background: "var(--surface-raised)",
              border: "1px solid var(--border)",
              borderRadius: 10,
            }}
          >
            <div
              style={{
                fontSize: 11,
                fontWeight: 600,
                letterSpacing: "0.06em",
                textTransform: "uppercase",
                color: "var(--text-dim)",
                marginBottom: 10,
              }}
            >
              Preview
            </div>
            <PanelPreview
              entity={entityName}
              spec={spec}
              chartKind={chartKind}
            />
          </div>

          {err && <div className="error-text">{err}</div>}
        </div>
        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>
            Cancel
          </button>
          <button
            className="btn btn-primary"
            disabled={busy || !title.trim()}
            onClick={() => void save()}
          >
            {busy ? "Saving…" : "Add panel"}
          </button>
        </div>
      </div>
    </div>
  );
}

function PanelPreview({
  entity,
  spec,
  chartKind,
}: {
  entity: string;
  spec: AggregateSpec;
  chartKind: string;
}) {
  const { data, loading, error } = db.useAggregate<Record<string, unknown>>(
    entity,
    spec,
  );
  if (error) return <div className="error-text">{error.message}</div>;
  if (loading && !data)
    return (
      <div style={{ color: "var(--text-dim)", fontSize: 12 }}>Loading…</div>
    );
  return <ChartRenderer kind={chartKind} data={data ?? []} spec={spec} />;
}

// ---------------------------------------------------------------------------
// Icons
// ---------------------------------------------------------------------------

function IconChart() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
      <path
        d="M4 20V10M10 20V4M16 20v-6M22 20H2"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function IconHome() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
      <path
        d="M3 12l9-9 9 9M5 10v10h14V10"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
function IconClipboard() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
      <rect x="6" y="4" width="12" height="16" rx="2" stroke="currentColor" strokeWidth="2" />
      <path d="M9 4h6M9 10h6M9 14h4" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
    </svg>
  );
}
function IconUsers() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
      <circle cx="9" cy="8" r="3" stroke="currentColor" strokeWidth="2" />
      <path d="M2 20c0-4 3-6 7-6s7 2 7 6" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
      <circle cx="17" cy="9" r="2.2" stroke="currentColor" strokeWidth="2" />
      <path d="M14 19c1-3 3-4 5-4" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
    </svg>
  );
}
function IconBox() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
      <path d="M3 8l9-5 9 5v8l-9 5-9-5V8z" stroke="currentColor" strokeWidth="2" strokeLinejoin="round" />
      <path d="M3 8l9 5 9-5M12 13v8" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
    </svg>
  );
}
function IconStack() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
      <path d="M3 7l9-4 9 4-9 4-9-4z" stroke="currentColor" strokeWidth="2" strokeLinejoin="round" />
      <path d="M3 12l9 4 9-4M3 17l9 4 9-4" stroke="currentColor" strokeWidth="2" strokeLinejoin="round" />
    </svg>
  );
}
function IconTeam() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
      <circle cx="12" cy="8" r="3.5" stroke="currentColor" strokeWidth="2" />
      <path d="M4 20c0-4 3.5-6 8-6s8 2 8 6" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
    </svg>
  );
}
function IconPlus() {
  return (
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none">
      <path d="M12 5v14M5 12h14" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" />
    </svg>
  );
}
