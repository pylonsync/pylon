import React, { useState } from "react";
import { Avatar } from "@/components/avatar";
import { ago } from "@/lib/format";
import { formatHandle } from "@/lib/handles";
import { cn } from "@/lib/utils";

export interface ContactRowView {
  id: string;
  name: string | null;
  handle: string;
  status: "allowed" | "blocked" | "unknown";
  optedOut: boolean;
  lastMessageAt: string | null;
  demo: boolean;
}

type Filter = "allowed" | "unknown" | "blocked";

const FILTERS: { value: Filter; label: string }[] = [
  { value: "allowed", label: "Allowed" },
  { value: "unknown", label: "Requests" },
  { value: "blocked", label: "Blocked" },
];

export function ContactsView({
  contacts,
  onAdd,
  onStatus,
  onOpen,
}: {
  contacts: ContactRowView[];
  onAdd: (handle: string, name: string) => Promise<void>;
  onStatus: (id: string, status: Filter) => void;
  onOpen: (id: string) => void;
}) {
  const [filter, setFilter] = useState<Filter>("allowed");
  const [handle, setHandle] = useState("");
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const shown = contacts.filter((c) => c.status === filter);

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-[760px] px-4 py-8 md:px-8 md:py-12">
        <h1 className="text-[28px] font-semibold tracking-[-0.025em]">Allowlist</h1>
        <p className="mt-1.5 max-w-[56ch] text-[14px] leading-relaxed text-muted-foreground">
          The assistant answers only the numbers and emails on this list. Everyone else lands in Requests, where you can
          allow or block them.
        </p>

        <form
          className="mt-7 rounded-[1.4rem] bg-black/[0.03] p-1.5 ring-1 ring-black/[0.04] dark:bg-white/[0.04]"
          onSubmit={async (e) => {
            e.preventDefault();
            setPending(true);
            setError(null);
            try {
              await onAdd(handle, name);
              setHandle("");
              setName("");
              setFilter("allowed");
            } catch (err) {
              setError(err instanceof Error ? err.message : "Could not add");
            } finally {
              setPending(false);
            }
          }}
        >
          <div className="flex flex-col gap-2 rounded-[calc(1.4rem-0.375rem)] bg-background p-3 shadow-[0_1px_2px_rgba(0,0,0,0.04)] sm:flex-row sm:items-end">
            <label className="flex-1">
              <span className="mb-1 block text-[12px] font-medium text-muted-foreground">Phone or email</span>
              <input
                value={handle}
                onChange={(e) => setHandle(e.target.value)}
                required
                placeholder="+1 512 555 0100"
                className="h-9 w-full rounded-lg border bg-background px-3 text-[14px] outline-none focus:ring-2 focus:ring-ring/30"
              />
            </label>
            <label className="flex-1">
              <span className="mb-1 block text-[12px] font-medium text-muted-foreground">Name</span>
              <input
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="Optional"
                className="h-9 w-full rounded-lg border bg-background px-3 text-[14px] outline-none focus:ring-2 focus:ring-ring/30"
              />
            </label>
            <button
              type="submit"
              disabled={pending}
              className="h-9 rounded-full bg-foreground px-5 text-[13px] font-medium text-background transition-transform duration-300 ease-[cubic-bezier(0.32,0.72,0,1)] active:scale-[0.98] disabled:opacity-50"
            >
              Allow
            </button>
          </div>
          {error ? <p className="px-3 pb-1.5 pt-2 text-[12px] text-destructive">{error}</p> : null}
        </form>

        <div className="mt-8 flex gap-1.5" role="tablist" aria-label="Filter contacts">
          {FILTERS.map((f) => {
            const count = contacts.filter((c) => c.status === f.value).length;
            return (
              <button
                key={f.value}
                type="button"
                role="tab"
                aria-selected={filter === f.value}
                onClick={() => setFilter(f.value)}
                className={cn(
                  "rounded-full px-3.5 py-1.5 text-[13px] font-medium transition-colors duration-300",
                  filter === f.value ? "bg-foreground text-background" : "text-muted-foreground hover:bg-accent",
                )}
              >
                {f.label}
                <span className="ml-1.5 tabular-nums opacity-60">{count}</span>
              </button>
            );
          })}
        </div>

        <ul className="mt-4 divide-y rounded-[1.2rem] bg-background ring-1 ring-border">
          {shown.length === 0 ? (
            <li className="px-4 py-10 text-center text-[13px] text-muted-foreground">
              {filter === "unknown" ? "No requests. Texts from numbers you haven't allowed show up here." : "Nobody here yet."}
            </li>
          ) : null}
          {shown.map((c) => (
            <li key={c.id} className="flex items-center gap-3 px-4 py-3">
              <Avatar name={c.name} handle={c.handle} size={36} />
              <button type="button" onClick={() => onOpen(c.id)} className="min-w-0 flex-1 text-left">
                <span className="block truncate text-[14px] font-medium">{c.name ?? formatHandle(c.handle)}</span>
                <span className="block truncate text-[12px] text-muted-foreground">
                  {c.name ? formatHandle(c.handle) : "No name"} · last text {ago(c.lastMessageAt)}
                  {c.optedOut ? " · texted STOP" : ""}
                  {c.demo ? " · sample" : ""}
                </span>
              </button>
              {c.status !== "allowed" ? (
                <button
                  type="button"
                  onClick={() => onStatus(c.id, "allowed")}
                  className="rounded-full bg-bubble-out px-3.5 py-1.5 text-[12px] font-medium text-white"
                >
                  Allow
                </button>
              ) : null}
              {c.status !== "blocked" ? (
                <button
                  type="button"
                  onClick={() => onStatus(c.id, "blocked")}
                  className="rounded-full px-3 py-1.5 text-[12px] font-medium text-muted-foreground ring-1 ring-border hover:text-destructive"
                >
                  Block
                </button>
              ) : (
                <button
                  type="button"
                  onClick={() => onStatus(c.id, "unknown")}
                  className="rounded-full px-3 py-1.5 text-[12px] font-medium text-muted-foreground ring-1 ring-border"
                >
                  Unblock
                </button>
              )}
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
