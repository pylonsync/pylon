"use client";

import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  callFn,
  db,
  getSync,
  resumeStream,
  storageKey,
  streamFn,
  useSyncStatus,
} from "@pylonsync/react";
import { EnsureGuest } from "@pylonsync/client";
import { siteConfig } from "@/lib/site.config";
import {
  cleanTitle,
  describeError,
  modelAllowedFor,
  isStaleStream,
  modelLabel,
  sortMessages,
  type ConversationRow,
  type MessageRow,
} from "@/lib/chat";
import type { ProviderStatus } from "@/lib/provider";
import { Sidebar, type Account } from "@/components/chat/sidebar";
import { Composer } from "@/components/chat/composer";
import { AssistantMessage, UserMessage } from "@/components/chat/message";
import { EmptyState } from "@/components/chat/empty-state";
import { ProviderSetup } from "@/components/chat/setup";
import { AlertIcon, ArrowDownIcon, CloseIcon, ComposeIcon, MenuIcon } from "@/components/chat/icons";

// The chat app. This file is the only one that talks to Pylon: live queries
// for conversations and messages, `streamFn("respond")` to send, and
// `resumeStream` to follow a reply that another tab started. Everything under
// components/chat receives plain props.

export interface ChatAppProps {
  initialConversationId: string | null;
  // From the SSR session: a signed-in account (not a guest) and its id.
  signedInUserId: string | null;
}

export function ChatApp(props: ChatAppProps) {
  // A visitor without a session gets a guest session before the app renders.
  return (
    <EnsureGuest fallback={<ShellSkeleton />}>
      <ChatShell {...props} />
    </EnsureGuest>
  );
}

// The reply this tab is streaming. `knownIds` holds the message ids that
// existed when it started, so the new rows can be told apart as they sync in.
interface LocalTurn {
  conversationId: string;
  userText: string | null;
  text: string;
  knownIds: Set<string>;
  stopped: boolean;
}

const MODEL_KEY = "lumen:model";

function readStoredModel(isGuest: boolean): string {
  try {
    const v = window.localStorage.getItem(MODEL_KEY);
    if (modelAllowedFor(v, isGuest)) return v as string;
  } catch {
    /* storage blocked */
  }
  return siteConfig.chat.defaultModel;
}

function pathFor(id: string | null): string {
  return id ? `/c/${encodeURIComponent(id)}` : "/";
}

function idFromPath(pathname: string): string | null {
  const m = pathname.match(/^\/c\/([^/]+)\/?$/);
  if (!m) return null;
  try {
    return decodeURIComponent(m[1]);
  } catch {
    return null;
  }
}

function ChatShell({ initialConversationId, signedInUserId }: ChatAppProps) {
  const { chat } = siteConfig;

  const { data: conversationRows, loading: conversationsLoading } =
    db.useQuery<ConversationRow>("Conversation");
  const [currentId, setCurrentId] = useState<string | null>(initialConversationId);
  const { data: messageRows } = db.useQuery<MessageRow>("Message", {
    where: { conversationId: currentId ?? "__none__" },
  });
  const messages = useMemo(() => sortMessages(messageRows), [messageRows]);
  const conversations = conversationRows;
  const current = conversations.find((c) => c.id === currentId) ?? null;

  const [provider, setProvider] = useState<ProviderStatus | null>(null);
  const [input, setInput] = useState("");
  const [model, setModel] = useState(chat.defaultModel);
  const [local, setLocal] = useState<LocalTurn | null>(null);
  // `turnRef` holds the live turn object the stream loop writes into;
  // `local` is a copy for rendering.
  const turnRef = useRef<LocalTurn | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [setupOpen, setSetupOpen] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  // The new-chat screen shows the composer mid-page on wide screens and docked
  // at the bottom on narrow ones; each placement has its own textarea.
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const dockTextareaRef = useRef<HTMLTextAreaElement>(null);
  const syncStatus = useSyncStatus(getSync());

  const startLocal = useCallback((turn: LocalTurn) => {
    turnRef.current = turn;
    setLocal({ ...turn });
  }, []);
  const renderLocal = useCallback((turn: LocalTurn) => {
    if (turnRef.current === turn) setLocal({ ...turn });
  }, []);
  const endLocal = useCallback((turn: LocalTurn) => {
    if (turnRef.current !== turn) return;
    turnRef.current = null;
    setLocal(null);
  }, []);

  /* ------------------------------ bootstrap ------------------------------ */

  useEffect(() => {
    setModel(readStoredModel(!signedInUserId));
    let cancelled = false;
    callFn<ProviderStatus>("assistantStatus")
      .then((s) => !cancelled && setProvider(s))
      .catch(() => !cancelled && setProvider({ configured: false, provider: null }));
    const t = setInterval(() => setNow(Date.now()), 60_000);
    return () => {
      cancelled = true;
      clearInterval(t);
    };
  }, []);

  // With no provider key, a visitor with an empty history gets the example
  // conversations from lib/examples.ts. The server re-checks both conditions.
  const seeded = useRef(false);
  useEffect(() => {
    if (seeded.current || provider?.configured !== false) return;
    if (conversationsLoading || conversations.length > 0) return;
    seeded.current = true;
    callFn("seedExamples").catch(() => {});
  }, [provider, conversationsLoading, conversations.length]);

  // Keep the URL and the open conversation in step, including Back/Forward.
  useEffect(() => {
    const onPop = () => {
      setCurrentId(idFromPath(window.location.pathname));
      setNotice(null);
    };
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);

  // A conversation deleted in another tab closes here too.
  useEffect(() => {
    if (currentId && !conversationsLoading && conversations.length > 0 && !current && !local) {
      openConversation(null, "replace");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentId, current, conversationsLoading, conversations.length, local]);

  function openConversation(id: string | null, mode: "push" | "replace" = "push") {
    setCurrentId(id);
    setNotice(null);
    setDrawerOpen(false);
    const path = pathFor(id);
    if (window.location.pathname !== path) {
      if (mode === "push") window.history.pushState(null, "", path);
      else window.history.replaceState(null, "", path);
    }
  }

  function newChat() {
    openConversation(null);
    setInput("");
    requestAnimationFrame(() => {
      const visible = [textareaRef.current, dockTextareaRef.current].find((el) => el && el.offsetParent !== null);
      visible?.focus();
    });
  }

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "o") {
        e.preventDefault();
        newChat();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function changeModel(id: string) {
    setModel(id);
    try {
      window.localStorage.setItem(MODEL_KEY, id);
    } catch {
      /* storage blocked */
    }
  }

  /* -------------------------------- send -------------------------------- */

  async function runTurn(
    conversationId: string,
    args: { content?: string; regenerate?: boolean },
    onStarted?: () => void,
  ): Promise<"done" | "rejected" | "failed"> {
    const turn: LocalTurn = {
      conversationId,
      userText: args.content ?? null,
      text: "",
      knownIds: new Set(messages.map((m) => m.id)),
      stopped: false,
    };
    startLocal(turn);
    onStarted?.();
    setNotice(null);

    try {
      const gen = streamFn("respond", { conversationId, model, ...args });
      let result: unknown;
      for (;;) {
        const step = await gen.next();
        if (step.done) {
          result = step.value;
          break;
        }
        if (turnRef.current !== turn || turn.stopped) {
          await gen.return(undefined);
          break;
        }
        turn.text += step.value;
        renderLocal(turn);
      }
      const r = result as { status?: string; errorCode?: string; message?: string } | undefined;
      if (r?.status === "rejected") {
        setNotice(r.message || describeError(r.errorCode).title);
        if (args.content) setInput((cur) => cur || args.content || "");
        return "rejected";
      }
      return "done";
    } catch {
      if (turnRef.current === turn && !turn.stopped) {
        setNotice("The connection dropped before the reply finished. It may still be completing; reload to check.");
      }
      return "failed";
    } finally {
      endLocal(turn);
    }
  }

  // Guards the gap between pressing Enter and the turn starting, so a double
  // submit cannot create two conversations. Once the turn starts, `turnRef`
  // takes over.
  const sendingRef = useRef(false);

  async function send(text: string) {
    const content = text.trim();
    if (!content || sendingRef.current) return;
    if (content.length > chat.maxInputChars) return;
    if (turnRef.current) {
      setNotice(describeError("BUSY").body);
      return;
    }
    sendingRef.current = true;
    setInput("");

    try {
      let conversationId = currentId;
      let created = false;
      if (!conversationId) {
        try {
          const { id } = await callFn<{ id: string }>("createConversation", { firstMessage: content });
          conversationId = id;
          created = true;
          openConversation(id, "push");
        } catch (err) {
          setInput(content);
          setNotice((err as { message?: string })?.message || "Could not start a new chat. Try again.");
          return;
        }
      }
      const outcome = await runTurn(conversationId, { content }, () => {
        sendingRef.current = false;
      });
      // A refused first message leaves nothing worth keeping in a new chat.
      if (outcome === "rejected" && created) {
        void callFn("deleteConversation", { conversationId }).catch(() => {});
        openConversation(null, "replace");
      }
    } finally {
      sendingRef.current = false;
    }
  }

  function regenerate() {
    if (!currentId || turnRef.current) return;
    void runTurn(currentId, { regenerate: true });
  }

  // Stop whichever reply is streaming in the open conversation: this tab's,
  // or one started in another tab. The server finds the streaming row by
  // conversation, so Stop works before the row has synced to this tab.
  async function stop() {
    if (!currentId) return;
    const conversationId = currentId;
    const row = messages.find((m) => m.role === "assistant" && m.status === "streaming");
    const turn = turnRef.current;
    let content = "";
    if (turn && turn.conversationId === conversationId) {
      turn.stopped = true;
      content = turn.text;
      endLocal(turn);
    } else if (row) {
      content = remoteText.current.get(row.id) ?? row.content;
    }
    for (let attempt = 0; attempt < 3; attempt++) {
      try {
        const r = await callFn<{ stopped: boolean }>("stopResponse", { conversationId, content });
        if (r.stopped || row) return;
      } catch {
        return;
      }
      // The server may not have inserted the reply row yet.
      await new Promise((res) => setTimeout(res, 700));
    }
  }

  // Text of replies being followed from another tab, for Stop.
  const remoteText = useRef(new Map<string, string>());

  /* ------------------------------ rendering ------------------------------ */

  const localHere = local && local.conversationId === currentId ? local : null;
  const streamingRow = messages.find(
    (m) => m.role === "assistant" && m.status === "streaming" && !isStaleStream(m, now),
  );
  const busy = Boolean(localHere) || Boolean(streamingRow);

  const newUserRowSynced =
    localHere?.userText != null &&
    messages.some((m) => m.role === "user" && !localHere.knownIds.has(m.id));
  const newAssistantRowSynced = localHere != null && messages.some((m) => m.role === "assistant" && !localHere.knownIds.has(m.id));

  const lastAssistant = [...messages].reverse().find((m) => m.role === "assistant");

  // The synced row is the source of truth for when a reply ends: finished,
  // failed, or stopped from another tab. This tab's turn ends when the row
  // leaves "streaming", without waiting for the stream connection to close.
  const localRow = localHere
    ? messages.find((m) => m.role === "assistant" && !localHere.knownIds.has(m.id))
    : undefined;
  useEffect(() => {
    const turn = turnRef.current;
    if (turn && localRow && localRow.status !== "streaming") {
      turn.stopped = true;
      endLocal(turn);
    }
  }, [localRow?.status, localRow, endLocal]);
  const empty = !currentId || (messages.length === 0 && !localHere);
  const isExample = Boolean(current?.example);
  const examples = conversations.filter((c) => c.example);

  const { data: me } = db.useQueryOne<{ email?: string }>("User", signedInUserId ?? "__none__");
  const account: Account = signedInUserId ? { kind: "user", email: me?.email ?? null } : { kind: "guest" };

  const scrollRef = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  const [showJump, setShowJump] = useState(false);

  function onScroll() {
    const el = scrollRef.current;
    if (!el) return;
    const distance = el.scrollHeight - el.scrollTop - el.clientHeight;
    pinned.current = distance < 120;
    setShowJump(distance > 400);
  }

  // Opening a conversation starts at its newest message; the new-chat screen
  // starts at the top.
  useEffect(() => {
    pinned.current = !empty;
    requestAnimationFrame(() => {
      const el = scrollRef.current;
      if (el) el.scrollTop = empty ? 0 : el.scrollHeight;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentId, empty]);

  const contentKey = `${messages.length}:${messages[messages.length - 1]?.content.length ?? 0}:${localHere?.text.length ?? 0}`;
  useEffect(() => {
    const el = scrollRef.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [contentKey]);

  function jumpToBottom() {
    const el = scrollRef.current;
    if (el) el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
  }

  const renderComposer = (ref: React.RefObject<HTMLTextAreaElement | null>, autoFocus = false) => (
    <Composer
      value={input}
      onChange={setInput}
      onSubmit={() => void send(input)}
      onStop={() => void stop()}
      busy={busy}
      placeholder={chat.inputPlaceholder}
      model={model}
      onModelChange={changeModel}
      isGuest={!signedInUserId}
      autoFocus={autoFocus}
      textareaRef={ref}
    />
  );

  const sidebar = (onClose?: () => void) => (
    <Sidebar
      conversations={conversations}
      currentId={currentId}
      now={now}
      account={account}
      onSelect={(id) => openConversation(id)}
      onNew={newChat}
      onRename={(id, title) => {
        const t = cleanTitle(title);
        if (t) void callFn("renameConversation", { conversationId: id, title: t }).catch(() => {});
      }}
      onDelete={(id) => {
        if (id === currentId) openConversation(null, "replace");
        void callFn("deleteConversation", { conversationId: id }).catch(() => {});
      }}
      onSignIn={() => window.location.assign(`/login?next=${encodeURIComponent(window.location.pathname)}`)}
      onSignOut={() => void signOut()}
      onClose={onClose}
    />
  );

  return (
    <div className="flex h-[100dvh] overflow-hidden bg-bg">
      {/* Desktop sidebar */}
      <aside className="hidden w-[272px] shrink-0 border-r border-line bg-paper md:block">
        {sidebar()}
      </aside>

      {/* Mobile drawer */}
      <div
        className={"fixed inset-0 z-40 md:hidden " + (drawerOpen ? "" : "pointer-events-none")}
        aria-hidden={!drawerOpen}
      >
        <div
          className={
            "absolute inset-0 bg-ink/25 backdrop-blur-[2px] transition-opacity duration-300 " +
            (drawerOpen ? "opacity-100" : "opacity-0")
          }
          onClick={() => setDrawerOpen(false)}
        />
        <aside
          className={
            "drawer absolute inset-y-0 left-0 w-[86%] max-w-[320px] bg-paper shadow-float " +
            (drawerOpen ? "translate-x-0" : "-translate-x-full")
          }
        >
          {sidebar(() => setDrawerOpen(false))}
        </aside>
      </div>

      <main className="relative flex min-w-0 flex-1 flex-col">
        <header className="z-10 flex h-14 shrink-0 items-center gap-2 px-3 sm:px-5">
          <button
            type="button"
            onClick={() => setDrawerOpen(true)}
            aria-label="Open sidebar"
            className="press flex size-9 items-center justify-center rounded-xl text-ink-2 hover:bg-hover md:hidden"
          >
            <MenuIcon size={20} />
          </button>
          <div className="min-w-0 flex-1">
            {current ? (
              <p className="truncate text-[14.5px] font-medium text-ink">
                {current.title}
                {isExample ? <span className="ml-2 text-[12.5px] font-normal text-ink-3">Example</span> : null}
              </p>
            ) : null}
          </div>
          {syncStatus !== "connected" && syncStatus !== "connecting" ? (
            <span className="rounded-full bg-hover px-2.5 py-1 text-[12px] text-ink-2">
              {syncStatus === "reconnecting" ? "Reconnecting" : "Offline"}
            </span>
          ) : null}
          <StatusChip provider={provider} model={model} onOpenSetup={() => setSetupOpen(true)} />
          <button
            type="button"
            onClick={newChat}
            aria-label="New chat"
            className="press flex size-9 items-center justify-center rounded-xl text-ink-2 hover:bg-hover md:hidden"
          >
            <ComposeIcon size={19} />
          </button>
        </header>

        <div ref={scrollRef} onScroll={onScroll} className="thread-scroll min-h-0 flex-1 overflow-y-auto">
          {empty ? (
            <EmptyState
              composer={renderComposer(textareaRef, true)}
              onPick={(p) => void send(p)}
              providerConfigured={provider ? provider.configured : null}
              exampleCount={examples.length}
              onOpenExample={examples[0] ? () => openConversation(examples[0].id) : undefined}
            />
          ) : (
            <div className="mx-auto w-full max-w-[46rem] px-4 pb-10 pt-4 sm:px-6 sm:pt-8">
              <div className="space-y-9">
                {messages.map((m) => {
                  if (m.role === "user") return <UserMessage key={m.id} content={m.content} />;
                  const isLast = m.id === lastAssistant?.id;
                  const canRegenerate = isLast && !busy && !isExample;
                  if (m.status === "streaming") {
                    if (isStaleStream(m, now)) {
                      return (
                        <AssistantMessage
                          key={m.id}
                          content={m.content}
                          status="interrupted"
                          model={m.model}
                          canRegenerate={canRegenerate}
                          onRegenerate={regenerate}
                        />
                      );
                    }
                    if (localHere) {
                      return (
                        <AssistantMessage
                          key={m.id}
                          content={localHere.text}
                          status="streaming"
                          model={m.model}
                          canRegenerate={false}
                        />
                      );
                    }
                    return <RemoteReply key={m.id} row={m} textStore={remoteText.current} />;
                  }
                  return (
                    <AssistantMessage
                      key={m.id}
                      content={m.content}
                      status={m.status ?? "done"}
                      errorCode={m.errorCode}
                      model={m.model}
                      canRegenerate={canRegenerate}
                      onRegenerate={regenerate}
                      providerConfigured={provider?.configured}
                    />
                  );
                })}
                {localHere && localHere.userText != null && !newUserRowSynced ? (
                  <UserMessage content={localHere.userText} pending />
                ) : null}
                {localHere && !newAssistantRowSynced ? (
                  <AssistantMessage content={localHere.text} status="streaming" canRegenerate={false} />
                ) : null}
              </div>
            </div>
          )}
        </div>

        <div className={"composer-dock relative shrink-0 px-3 pb-3 sm:px-6 sm:pb-5 " + (empty ? "md:hidden" : "")}>
            {showJump ? (
              <button
                type="button"
                onClick={jumpToBottom}
                aria-label="Scroll to latest"
                className="press absolute -top-12 left-1/2 flex size-9 -translate-x-1/2 items-center justify-center rounded-full bg-surface text-ink-2 shadow-float ring-1 ring-line"
              >
                <ArrowDownIcon size={17} />
              </button>
            ) : null}
            <div className="mx-auto w-full max-w-[46rem]">
              {notice ? <Notice text={notice} onDismiss={() => setNotice(null)} /> : null}
              {isExample && !empty ? (
                <ExampleBar onNew={newChat} />
              ) : (
                <>
                  {renderComposer(empty ? dockTextareaRef : textareaRef)}
                  <p className="mt-2 text-center text-[12px] text-ink-3">
                    {siteConfig.brand.name} can make mistakes. Check important information.
                  </p>
                </>
              )}
            </div>
          </div>
        {empty && notice ? (
          <div className="hidden shrink-0 px-3 pb-4 md:block">
            <div className="mx-auto max-w-[46rem]">
              <Notice text={notice} onDismiss={() => setNotice(null)} />
            </div>
          </div>
        ) : null}
      </main>

      {setupOpen ? <SetupDialog onClose={() => setSetupOpen(false)} /> : null}
    </div>
  );
}

// A reply that another tab (or this tab before a reload) is streaming.
// Attaches to its resumable stream and shows tokens as they arrive; the row
// itself switches to "done" when the reply is stored.
function RemoteReply({ row, textStore }: { row: MessageRow; textStore: Map<string, string> }) {
  const [text, setText] = useState(row.content);

  useEffect(() => {
    if (!row.streamId) return;
    let cancelled = false;
    let acc = "";
    const gen = resumeStream(row.streamId);
    (async () => {
      try {
        for await (const chunk of gen) {
          if (cancelled) break;
          acc += chunk;
          textStore.set(row.id, acc);
          setText(acc);
        }
      } catch {
        /* the stream expired or the reply finished; the synced row takes over */
      }
    })();
    return () => {
      cancelled = true;
      textStore.delete(row.id);
      void gen.return(undefined);
    };
  }, [row.id, row.streamId, textStore]);

  return <AssistantMessage content={text} status="streaming" model={row.model} canRegenerate={false} />;
}

function StatusChip({
  provider,
  model,
  onOpenSetup,
}: {
  provider: ProviderStatus | null;
  model: string;
  onOpenSetup: () => void;
}) {
  if (!provider) return null;
  if (!provider.configured) {
    return (
      <button
        type="button"
        onClick={onOpenSetup}
        className="press inline-flex h-8 items-center gap-2 rounded-full bg-warn-soft px-3 text-[12.5px] font-medium text-warn ring-1 ring-warn/15 transition-colors duration-200 hover:bg-warn/10"
      >
        <span className="size-1.5 rounded-full bg-warn" />
        No API key
      </button>
    );
  }
  return (
    <span className="hidden h-8 items-center gap-2 rounded-full px-3 text-[12.5px] text-ink-2 ring-1 ring-line sm:inline-flex">
      <span className="size-1.5 rounded-full bg-ok" />
      {modelLabel(model)}
    </span>
  );
}

function Notice({ text, onDismiss }: { text: string; onDismiss: () => void }) {
  return (
    <div className="msg-in mb-2 flex items-start gap-3 rounded-2xl bg-warn-soft px-4 py-2.5 ring-1 ring-warn/15" role="alert">
      <AlertIcon size={17} className="mt-0.5 shrink-0 text-warn" />
      <p className="flex-1 text-[13.5px] leading-relaxed text-ink">{text}</p>
      <button type="button" onClick={onDismiss} aria-label="Dismiss" className="text-ink-3 hover:text-ink">
        <CloseIcon size={16} />
      </button>
    </div>
  );
}

function ExampleBar({ onNew }: { onNew: () => void }) {
  return (
    <div className="bezel">
      <div className="bezel-core flex flex-col gap-3 px-4 py-3.5 sm:flex-row sm:items-center">
        <p className="flex-1 text-[13.5px] leading-relaxed text-ink-2">
          This is an example conversation written for the template. No model generated it, and it is read-only.
        </p>
        <button
          type="button"
          onClick={onNew}
          className="press inline-flex h-9 shrink-0 items-center justify-center gap-2 rounded-full bg-ink px-4 text-[13.5px] font-medium text-bg transition-opacity duration-200 hover:opacity-90"
        >
          <ComposeIcon size={16} />
          New chat
        </button>
      </div>
    </div>
  );
}

function SetupDialog({ onClose }: { onClose: () => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div className="fixed inset-0 z-50 flex items-end justify-center p-3 sm:items-center sm:p-6" role="dialog" aria-modal="true" aria-label="Add a model provider key">
      <div className="absolute inset-0 bg-ink/30 backdrop-blur-sm" onClick={onClose} />
      <div className="dialog-in relative w-full max-w-lg">
        <button
          type="button"
          onClick={onClose}
          aria-label="Close"
          className="absolute right-4 top-4 z-10 flex size-8 items-center justify-center rounded-lg text-ink-3 hover:bg-hover hover:text-ink"
        >
          <CloseIcon size={17} />
        </button>
        <ProviderSetup />
      </div>
    </div>
  );
}

async function signOut() {
  try {
    await getSync().signOut();
  } catch {
    /* the session may already be gone */
  }
  try {
    window.localStorage.removeItem(storageKey("token"));
  } catch {
    /* storage blocked */
  }
  window.location.assign("/");
}

function ShellSkeleton() {
  return (
    <div className="flex h-[100dvh] bg-bg">
      <aside className="hidden w-[272px] shrink-0 border-r border-line bg-paper md:block" />
      <main className="flex-1" />
    </div>
  );
}
