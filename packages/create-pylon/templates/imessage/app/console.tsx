"use client";

import React, { useEffect, useMemo, useRef, useState } from "react";
import { callFn, db, useRouter } from "@pylonsync/react";
import { useAuth } from "@pylonsync/client";
import { ContactPanel, type PanelActivity } from "@/components/contact-panel";
import { ContactsView, type ContactRowView } from "@/components/contacts-view";
import { ConversationList, type ConversationSummary } from "@/components/conversation-list";
import { NavRail, type View } from "@/components/nav-rail";
import { setupIssueCount, SetupView, type SetupStatus, type SettingsView, type TransportStateView } from "@/components/setup-view";
import { Thread, type Notice } from "@/components/thread";
import { describeToolCall, toolRefusal, type ThreadMessage } from "@/lib/format";
import { DEFAULT_SETTINGS } from "@/lib/gate";
import { cn } from "@/lib/utils";

interface ContactRow {
  id: string;
  handle: string;
  displayName?: string | null;
  status: string;
  optedOut?: boolean | null;
  lastMessageAt?: string | null;
  demo?: boolean | null;
}
interface ConversationRow {
  id: string;
  contactId: string;
  handle: string;
  lastMessageAt?: string | null;
  lastPreview?: string | null;
  agentRunId?: string | null;
  turnState?: string | null;
  lastError?: string | null;
  demo?: boolean | null;
}
interface MessageRow extends ThreadMessage {
  conversationId: string;
}
interface NoteRow {
  id: string;
  contactId: string;
  text: string;
  createdAt: string;
}
interface ReminderRow {
  id: string;
  contactId: string;
  text: string;
  dueAt: string;
  status: string;
}
interface AgentRunRow {
  id: string;
  status: string;
  steps?: number | null;
  error?: string | null;
  updatedAt: string;
  context?: { conversationId?: string } | null;
}
interface AgentMessageRow {
  id: string;
  runId: string;
  role: string;
  content: unknown;
  createdAt: string;
}
interface SettingsRow extends Partial<SettingsView> {
  id: string;
  key: string;
}

const UNANSWERED = new Set(["awaiting_setup", "failed", "rate_limited"]);

function errorMessage(err: unknown): string {
  if (err && typeof err === "object" && "message" in err) return String((err as { message: unknown }).message);
  return "Something went wrong";
}

/**
 * The dashboard, and the only module that touches `db` / `callFn`. Every
 * `db.useQuery` below is a live subscription: a text that arrives through the
 * webhook or the relay shows up in the list and the open thread without a
 * reload, and so does the assistant's reply.
 */
export function Console({ view, initialConversationId }: { view: View; initialConversationId: string | null }) {
  const router = useRouter();
  const { signOut } = useAuth();
  const { data: contacts } = db.useQuery<ContactRow>("Contact");
  const { data: conversations, loading } = db.useQuery<ConversationRow>("Conversation");
  const { data: messages } = db.useQuery<MessageRow>("Message");
  const { data: notes } = db.useQuery<NoteRow>("Note");
  const { data: reminders } = db.useQuery<ReminderRow>("Reminder");
  const { data: runs } = db.useQuery<AgentRunRow>("AgentRun");
  const { data: agentMessages } = db.useQuery<AgentMessageRow>("AgentMessage");
  const { data: settingsRows } = db.useQuery<SettingsRow>("Settings");
  const { data: transportRows } = db.useQuery<TransportStateView & { id: string }>("TransportState");

  const [status, setStatus] = useState<SetupStatus | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(initialConversationId);
  const [query, setQuery] = useState("");
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const seeded = useRef(false);

  useEffect(() => {
    callFn<SetupStatus>("setupStatus", {}).then(setStatus, () => setStatus(null));
  }, []);

  // A fresh install gets the sample threads once, so the dashboard never opens empty.
  useEffect(() => {
    if (loading || seeded.current) return;
    seeded.current = true;
    if ((conversations ?? []).length > 0) return;
    callFn("seedDemo", {}).catch(() => {
      seeded.current = false;
    });
  }, [loading, conversations]);

  const contactById = useMemo(() => new Map((contacts ?? []).map((c) => [c.id, c] as const)), [contacts]);

  const summaries: ConversationSummary[] = useMemo(() => {
    const unansweredByConversation = new Map<string, number>();
    for (const m of messages ?? []) {
      if (m.direction === "in" && UNANSWERED.has(m.status)) {
        unansweredByConversation.set(m.conversationId, (unansweredByConversation.get(m.conversationId) ?? 0) + 1);
      }
    }
    const q = query.trim().toLowerCase();
    return (conversations ?? [])
      .map((c) => {
        const contact = contactById.get(c.contactId);
        const status = (contact?.status ?? "unknown") as ConversationSummary["status"];
        return {
          id: c.id,
          name: contact?.displayName ?? null,
          handle: c.handle,
          preview: c.lastPreview ?? "",
          lastMessageAt: c.lastMessageAt ?? null,
          status,
          thinking: c.turnState === "running",
          needsAttention: status === "allowed" && (unansweredByConversation.get(c.id) ?? 0) > 0,
          demo: Boolean(c.demo),
        };
      })
      .filter((c) => !q || (c.name ?? "").toLowerCase().includes(q) || c.handle.includes(q) || c.preview.toLowerCase().includes(q))
      .sort((a, b) => ((a.lastMessageAt ?? "") < (b.lastMessageAt ?? "") ? 1 : -1));
  }, [conversations, contactById, messages, query]);

  // Desktop opens the newest conversation; phones start on the list.
  useEffect(() => {
    if (view !== "messages" || selectedId || summaries.length === 0) return;
    if (typeof window !== "undefined" && window.matchMedia("(min-width: 768px)").matches) {
      setSelectedId(summaries[0].id);
    }
  }, [view, selectedId, summaries]);

  function select(id: string | null) {
    setSelectedId(id);
    setDetailsOpen(false);
    // Keep the URL shareable without a navigation round trip.
    window.history.replaceState(null, "", id ? `/?c=${encodeURIComponent(id)}` : "/");
  }

  async function run<T>(fn: string, args: Record<string, unknown>): Promise<T> {
    try {
      return await callFn<T>(fn, args);
    } catch (err) {
      const message = errorMessage(err);
      setToast(message);
      setTimeout(() => setToast(null), 4000);
      throw new Error(message);
    }
  }

  const settingsRow = (settingsRows ?? []).find((s) => s.key === "main");
  const settings: SettingsView = {
    ownerName: settingsRow?.ownerName ?? "",
    timezone: settingsRow?.timezone ?? DEFAULT_SETTINGS.timezone,
    unknownSenderPolicy: settingsRow?.unknownSenderPolicy ?? DEFAULT_SETTINGS.unknownSenderPolicy,
    unknownSenderReply: settingsRow?.unknownSenderReply ?? DEFAULT_SETTINGS.unknownSenderReply,
    rateLimitPerHour: settingsRow?.rateLimitPerHour ?? DEFAULT_SETTINGS.rateLimitPerHour,
    paused: Boolean(settingsRow?.paused),
  };
  const transport = (transportRows ?? []).find((t) => t.name === status?.transport) ?? null;
  const requests = (contacts ?? []).filter((c) => c.status === "unknown").length;

  const nav = (
    <NavRail
      view={view}
      setupIssues={setupIssueCount(status)}
      requests={requests}
      onSignOut={() => {
        void signOut();
        window.location.assign("/login");
      }}
    />
  );

  let body: React.ReactNode;
  if (view === "contacts") {
    const rows: ContactRowView[] = (contacts ?? [])
      .map((c) => ({
        id: c.id,
        name: c.displayName ?? null,
        handle: c.handle,
        status: (c.status ?? "unknown") as ContactRowView["status"],
        optedOut: Boolean(c.optedOut),
        lastMessageAt: c.lastMessageAt ?? null,
        demo: Boolean(c.demo),
      }))
      .sort((a, b) => ((a.lastMessageAt ?? "") < (b.lastMessageAt ?? "") ? 1 : -1));
    body = (
      <ContactsView
        contacts={rows}
        onAdd={async (handle, name) => {
          await run("saveContact", { handle, displayName: name, status: "allowed" });
        }}
        onStatus={(id, s) => void run("setContactStatus", { contactId: id, status: s }).catch(() => {})}
        onOpen={(contactId) => {
          const conv = (conversations ?? []).find((c) => c.contactId === contactId);
          router.push(conv ? `/?c=${encodeURIComponent(conv.id)}` : "/");
        }}
      />
    );
  } else if (view === "setup") {
    body = (
      <SetupView
        status={status}
        transport={transport}
        settings={settings}
        hasDemo={(conversations ?? []).some((c) => c.demo)}
        onSaveSettings={async (patch) => {
          await run("updateSettings", patch);
        }}
        onClearDemo={async () => {
          await run("clearDemo", {});
        }}
      />
    );
  } else {
    const conversation = (conversations ?? []).find((c) => c.id === selectedId) ?? null;
    const contact = conversation ? contactById.get(conversation.contactId) : undefined;
    const threadMessages = conversation ? (messages ?? []).filter((m) => m.conversationId === conversation.id) : [];
    const unanswered = threadMessages.filter((m) => m.direction === "in" && UNANSWERED.has(m.status)).length;
    const runRow = conversation ? (runs ?? []).find((r) => r.id === conversation.agentRunId || r.context?.conversationId === conversation.id) : undefined;
    const activity: PanelActivity[] = [];
    if (conversation) {
      const runIds = new Set((runs ?? []).filter((r) => r.id === conversation.agentRunId || r.context?.conversationId === conversation.id).map((r) => r.id));
      const refusals = new Map<string, string | null>();
      for (const m of agentMessages ?? []) {
        if (!runIds.has(m.runId) || m.role !== "user" || !Array.isArray(m.content)) continue;
        for (const block of m.content as { type?: string; tool_use_id?: string; content?: unknown; is_error?: boolean }[]) {
          if (block.type === "tool_result" && block.tool_use_id) {
            refusals.set(block.tool_use_id, toolRefusal(block.content, block.is_error === true));
          }
        }
      }
      for (const m of agentMessages ?? []) {
        if (!runIds.has(m.runId) || m.role !== "assistant" || !Array.isArray(m.content)) continue;
        for (const block of m.content as { type?: string; id?: string; name?: string; input?: Record<string, unknown> }[]) {
          if (block.type === "tool_use" && block.name) {
            const refused = block.id ? refusals.get(block.id) : null;
            const label = describeToolCall(block.name, block.input ?? {});
            activity.push({
              id: block.id ?? `${m.id}-${activity.length}`,
              label: refused ? `Refused: ${label.charAt(0).toLowerCase()}${label.slice(1)}. ${refused}` : label,
              at: m.createdAt,
            });
          }
        }
      }
      activity.sort((a, b) => (a.at < b.at ? 1 : -1)).splice(8);
    }

    let notice: Notice | null = null;
    if (conversation && contact) {
      if (status && !status.transport) {
        notice = { tone: "warn", title: "No transport", body: <>Set IMESSAGE_TRANSPORT to sendblue or relay. See Setup.</> };
      } else if (status && !status.provider.configured && contact.status === "allowed") {
        notice = {
          tone: "warn",
          title: "Replies are off",
          body: <>No model key is set, so new texts wait here unanswered. Add ANTHROPIC_API_KEY to the server environment, then answer them from the contact details.</>,
        };
      } else if (settings.paused && contact.status === "allowed") {
        notice = { tone: "muted", title: "Assistant paused", body: <>Texts are stored. Turn replies back on in Setup.</> };
      } else if (conversation.lastError && unanswered > 0) {
        notice = { tone: "warn", title: "Last reply failed", body: conversation.lastError };
      }
    }

    const panel = (onClose?: () => void) =>
      conversation && contact ? (
        <ContactPanel
          name={contact.displayName ?? null}
          handle={contact.handle}
          status={(contact.status ?? "unknown") as "allowed" | "unknown" | "blocked"}
          optedOut={Boolean(contact.optedOut)}
          unanswered={unanswered}
          notes={(notes ?? []).filter((n) => n.contactId === contact.id).sort((a, b) => (a.createdAt < b.createdAt ? 1 : -1))}
          reminders={(reminders ?? [])
            .filter((r) => r.contactId === contact.id && r.status !== "cancelled")
            .sort((a, b) => (a.dueAt < b.dueAt ? -1 : 1))}
          run={runRow ? { status: runRow.status, steps: Number(runRow.steps) || 0, updatedAt: runRow.updatedAt, error: runRow.error ?? null } : null}
          activity={activity}
          onClose={onClose}
          onStatus={(s) => void run("setContactStatus", { contactId: contact.id, status: s }).catch(() => {})}
          onRename={(name) =>
            void run("saveContact", { handle: contact.handle, displayName: name, status: contact.status }).catch(() => {})
          }
          onAnswerNow={() => void run("answerNow", { conversationId: conversation.id }).catch(() => {})}
          onDeleteNote={(id) => void run("deleteNote", { noteId: id }).catch(() => {})}
          onCancelReminder={(id) => void run("cancelReminder", { reminderId: id }).catch(() => {})}
        />
      ) : null;

    body = (
      <div className="flex h-full min-h-0">
        <div
          className={cn(
            "h-full min-h-0 w-full flex-col border-r bg-rail/50 pt-3 md:flex md:w-[320px] md:shrink-0",
            conversation ? "hidden" : "flex",
          )}
        >
          <div className="flex items-baseline justify-between px-4 pb-2">
            <h1 className="text-[22px] font-semibold tracking-[-0.025em]">Messages</h1>
            <span className="text-[12px] text-muted-foreground">{summaries.length} threads</span>
          </div>
          <ConversationList conversations={summaries} selectedId={selectedId} query={query} onQuery={setQuery} onSelect={select} />
        </div>
        {conversation && contact ? (
          <>
            <div className="h-full min-h-0 min-w-0 flex-1">
              <Thread
                key={conversation.id}
                contact={{
                  name: contact.displayName ?? null,
                  handle: contact.handle,
                  status: (contact.status ?? "unknown") as "allowed" | "unknown" | "blocked",
                  optedOut: Boolean(contact.optedOut),
                }}
                messages={threadMessages}
                thinking={conversation.turnState === "running"}
                notice={notice}
                demo={Boolean(conversation.demo)}
                onBack={() => select(null)}
                onToggleDetails={() => setDetailsOpen((o) => !o)}
                onSend={async (text) => {
                  await run("sendOwnerMessage", { conversationId: conversation.id, text });
                }}
              />
            </div>
            <div className="hidden h-full w-[300px] shrink-0 border-l xl:block">{panel()}</div>
            {detailsOpen ? (
              <div className="fixed inset-0 z-30 flex justify-end bg-black/20 xl:hidden" onClick={() => setDetailsOpen(false)}>
                <div className="h-full w-[min(340px,92vw)] bg-background shadow-2xl" onClick={(e) => e.stopPropagation()}>
                  {panel(() => setDetailsOpen(false))}
                </div>
              </div>
            ) : null}
          </>
        ) : (
          <div className="hidden flex-1 items-center justify-center bg-background text-[14px] text-muted-foreground md:flex">
            {loading ? "Loading…" : summaries.length ? "Select a conversation." : "No conversations yet."}
          </div>
        )}
      </div>
    );
  }

  return (
    <div className="flex h-[100dvh] flex-col overflow-hidden md:flex-row">
      {nav}
      <main className="min-h-0 min-w-0 flex-1">{body}</main>
      {toast ? (
        <div role="alert" className="fixed bottom-20 left-1/2 z-40 -translate-x-1/2 rounded-full bg-foreground px-4 py-2 text-[13px] text-background shadow-lg md:bottom-6">
          {toast}
        </div>
      ) : null}
    </div>
  );
}
