import React, { useState } from "react";
import { Bell, NotebookPen, Sparkles, X } from "lucide-react";
import { Avatar } from "@/components/avatar";
import { ago } from "@/lib/format";
import { formatHandle } from "@/lib/handles";
import { cn } from "@/lib/utils";

export interface PanelNote {
  id: string;
  text: string;
  createdAt: string;
}
export interface PanelReminder {
  id: string;
  text: string;
  dueAt: string;
  status: string;
}
export interface PanelActivity {
  id: string;
  label: string;
  at: string;
}
export interface PanelRun {
  status: string;
  steps: number;
  updatedAt: string;
  error: string | null;
}

type Status = "allowed" | "unknown" | "blocked";

const STATUS_OPTIONS: { value: Status; label: string }[] = [
  { value: "allowed", label: "Allowed" },
  { value: "unknown", label: "Not listed" },
  { value: "blocked", label: "Blocked" },
];

export function ContactPanel({
  name,
  handle,
  status,
  optedOut,
  unanswered,
  notes,
  reminders,
  run,
  activity,
  onClose,
  onStatus,
  onRename,
  onAnswerNow,
  onDeleteNote,
  onCancelReminder,
}: {
  name: string | null;
  handle: string;
  status: Status;
  optedOut: boolean;
  unanswered: number;
  notes: PanelNote[];
  reminders: PanelReminder[];
  run: PanelRun | null;
  activity: PanelActivity[];
  onClose?: () => void;
  onStatus: (status: Status) => void;
  onRename: (name: string) => void;
  onAnswerNow: () => void;
  onDeleteNote: (id: string) => void;
  onCancelReminder: (id: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(name ?? "");

  return (
    <aside className="flex h-full min-h-0 flex-col overflow-y-auto bg-rail/60 px-4 pb-6 pt-4" aria-label="Contact details">
      {onClose ? (
        <button
          type="button"
          onClick={onClose}
          className="ml-auto rounded-full p-1.5 text-muted-foreground hover:bg-accent"
          aria-label="Close details"
        >
          <X className="size-4" strokeWidth={1.5} />
        </button>
      ) : null}
      <div className="flex flex-col items-center pt-2 text-center">
        <Avatar name={name} handle={handle} size={64} />
        {editing ? (
          <form
            className="mt-3 flex w-full gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              onRename(draft);
              setEditing(false);
            }}
          >
            <input
              autoFocus
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              aria-label="Name"
              className="h-8 min-w-0 flex-1 rounded-lg border bg-background px-2.5 text-[14px] outline-none focus:ring-2 focus:ring-ring/30"
            />
            <button type="submit" className="rounded-lg bg-foreground px-3 text-[13px] font-medium text-background">
              Save
            </button>
          </form>
        ) : (
          <button
            type="button"
            onClick={() => {
              setDraft(name ?? "");
              setEditing(true);
            }}
            className="mt-3 rounded-md px-1 text-[17px] font-semibold tracking-[-0.015em] hover:bg-accent"
          >
            {name ?? "Add a name"}
          </button>
        )}
        <p className="mt-0.5 font-mono text-[12px] text-muted-foreground">{formatHandle(handle)}</p>
      </div>

      <Card className="mt-5">
        <p className="text-[13px] font-medium">Assistant replies to this person</p>
        <div className="mt-2.5 grid grid-cols-3 rounded-[0.65rem] bg-black/[0.05] p-0.5 dark:bg-white/[0.06]" role="radiogroup" aria-label="Allowlist status">
          {STATUS_OPTIONS.map((o) => (
            <button
              key={o.value}
              type="button"
              role="radio"
              aria-checked={status === o.value}
              onClick={() => onStatus(o.value)}
              className={cn(
                "rounded-[0.55rem] py-1.5 text-[12px] font-medium transition-all duration-300 ease-[cubic-bezier(0.32,0.72,0,1)]",
                status === o.value
                  ? "bg-background text-foreground shadow-[0_1px_2px_rgba(0,0,0,0.08),0_0_0_0.5px_rgba(0,0,0,0.06)]"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {o.label}
            </button>
          ))}
        </div>
        <p className="mt-2 text-[12px] leading-snug text-muted-foreground">
          {optedOut
            ? "This person texted STOP. Nothing is sent to them until they text START."
            : status === "allowed"
              ? "Texts from this number get an answer from the assistant."
              : status === "blocked"
                ? "Texts are stored and never answered."
                : "Texts are stored for you to review. The assistant does not answer."}
        </p>
        {unanswered > 0 && status === "allowed" && !optedOut ? (
          <button
            type="button"
            onClick={onAnswerNow}
            className="mt-3 w-full rounded-full bg-bubble-out py-2 text-[13px] font-medium text-white transition-transform duration-300 ease-[cubic-bezier(0.32,0.72,0,1)] active:scale-[0.98]"
          >
            Answer {unanswered} unanswered text{unanswered === 1 ? "" : "s"}
          </button>
        ) : null}
      </Card>

      <Section icon={<Sparkles className="size-3.5" strokeWidth={1.5} />} title="Assistant activity">
        {run ? (
          <p className="mb-2 text-[12px] text-muted-foreground">
            Last run {run.status === "running" ? "is writing now" : run.status} · {run.steps} step
            {run.steps === 1 ? "" : "s"} · {ago(run.updatedAt)}
            {run.error ? <span className="mt-1 block text-destructive">{run.error}</span> : null}
          </p>
        ) : null}
        {activity.length === 0 ? (
          <Empty>No tool use yet.</Empty>
        ) : (
          <ul className="space-y-1.5">
            {activity.map((a) => (
              <li key={a.id} className="text-[13px] leading-snug">
                {a.label}
                <span className="ml-1.5 text-[11px] text-muted-foreground">{ago(a.at)}</span>
              </li>
            ))}
          </ul>
        )}
      </Section>

      <Section icon={<NotebookPen className="size-3.5" strokeWidth={1.5} />} title={`Notes${notes.length ? ` (${notes.length})` : ""}`}>
        {notes.length === 0 ? (
          <Empty>The assistant saves facts here when this person mentions them.</Empty>
        ) : (
          <ul className="space-y-1">
            {notes.map((n) => (
              <li key={n.id} className="group flex items-start gap-2 rounded-lg py-1 text-[13px] leading-snug">
                <span className="flex-1">{n.text}</span>
                <button
                  type="button"
                  onClick={() => onDeleteNote(n.id)}
                  className="rounded p-0.5 text-muted-foreground opacity-0 transition-opacity hover:text-destructive group-hover:opacity-100 focus:opacity-100"
                  aria-label={`Delete note: ${n.text}`}
                >
                  <X className="size-3.5" strokeWidth={1.5} />
                </button>
              </li>
            ))}
          </ul>
        )}
      </Section>

      <Section icon={<Bell className="size-3.5" strokeWidth={1.5} />} title="Reminders">
        {reminders.length === 0 ? (
          <Empty>None scheduled.</Empty>
        ) : (
          <ul className="space-y-2">
            {reminders.map((r) => (
              <li key={r.id} className="rounded-lg bg-background/80 px-2.5 py-2 text-[13px] leading-snug ring-1 ring-border">
                <p className="text-[12px] font-medium text-muted-foreground">
                  {new Date(r.dueAt).toLocaleString("en-US", { weekday: "short", month: "short", day: "numeric", hour: "numeric", minute: "2-digit" })}
                  {r.status !== "scheduled" ? ` · ${r.status}` : ""}
                </p>
                <p className="mt-0.5">{r.text}</p>
                {r.status === "scheduled" ? (
                  <button
                    type="button"
                    onClick={() => onCancelReminder(r.id)}
                    className="mt-1 text-[12px] font-medium text-destructive hover:underline"
                  >
                    Cancel
                  </button>
                ) : null}
              </li>
            ))}
          </ul>
        )}
      </Section>
    </aside>
  );
}

function Card({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <div className={cn("rounded-[1.1rem] bg-black/[0.03] p-1 ring-1 ring-black/[0.04] dark:bg-white/[0.04] dark:ring-white/[0.06]", className)}>
      <div className="rounded-[calc(1.1rem-0.25rem)] bg-background p-3 shadow-[inset_0_1px_0_rgba(255,255,255,0.6),0_1px_2px_rgba(0,0,0,0.03)]">
        {children}
      </div>
    </div>
  );
}

function Section({ icon, title, children }: { icon: React.ReactNode; title: string; children: React.ReactNode }) {
  return (
    <section className="mt-6 px-1">
      <h2 className="mb-2 flex items-center gap-1.5 text-[13px] font-semibold text-foreground">
        <span className="text-muted-foreground">{icon}</span>
        {title}
      </h2>
      {children}
    </section>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return <p className="text-[12px] leading-snug text-muted-foreground">{children}</p>;
}
