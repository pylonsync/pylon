"use client";

import React, { useEffect, useRef, useState } from "react";
import { db } from "@pylonsync/react";
import { siteConfig } from "@/lib/site.config";

// The chat app — a client island, rendered only for a SIGNED-IN user (the page
// redirects anyone else to /login). Conversations + messages are sync-backed
// owner-scoped entities (`db.useQuery`), so your history is private to your
// account and stays in lockstep across your tabs + devices. Sending streams
// tokens from the built-in `POST /api/ai/stream` (SSE) — your PYLON_AI_API_KEY
// never reaches the browser.
//
// All state lives in <ChatInner> (which owns `currentId`) — the thread is a
// presentational child. That's deliberate: creating a conversation mid-send
// changes `currentId`, and if the thread remounted on that change it would kill
// the in-flight stream. One owner, no remounts.

interface ConversationRow {
  id: string;
  userId: string;
  title: string;
  createdAt: string;
}
interface MessageRow {
  id: string;
  conversationId: string;
  userId: string;
  role: string;
  content: string;
  createdAt: string;
}

export function ChatApp() {
  return <ChatInner />;
}

function ChatInner() {
  const { chat } = siteConfig;
  const { data: conversations } = db.useQuery<ConversationRow>("Conversation", {
    orderBy: { createdAt: "desc" },
  });
  const [currentId, setCurrentId] = useState<string | null>(null);
  const { data: messages } = db.useQuery<MessageRow>("Message", {
    where: { conversationId: currentId ?? "__none__" },
    orderBy: { createdAt: "asc" },
  });

  const [streaming, setStreaming] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [input, setInput] = useState("");
  const [model, setModel] = useState(chat.defaultModel);
  const scrollRef = useRef<HTMLDivElement>(null);
  const initialized = useRef(false);

  // On first load, drop into the most recent conversation (if any).
  useEffect(() => {
    if (!initialized.current && conversations.length > 0) {
      initialized.current = true;
      setCurrentId(conversations[0].id);
    }
  }, [conversations]);

  // Keep the latest turn in view as content streams in.
  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, [messages.length, streaming]);

  function selectConversation(id: string) {
    setCurrentId(id);
    setStreaming(null);
    setNotice(null);
  }
  function newChat() {
    setCurrentId(null);
    setStreaming(null);
    setNotice(null);
  }

  async function send(text: string) {
    const trimmed = text.trim();
    if (!trimmed || sending) return;
    setSending(true);
    setNotice(null);
    setInput("");

    // Snapshot the history BEFORE the async work (messages is for the current
    // conversation; empty for a brand-new chat).
    const history = messages.map((m) => ({ role: m.role, content: m.content }));

    // Make sure we have a conversation to attach to.
    let convId = currentId;
    if (!convId) {
      convId = await db.insert("Conversation", { title: trimmed.slice(0, 48) });
      setCurrentId(convId);
    }

    // Persist the user's turn (optimistic — paints immediately).
    await db.insert("Message", { conversationId: convId, role: "user", content: trimmed });

    const payload = [
      { role: "system", content: chat.systemPrompt },
      ...history,
      { role: "user", content: trimmed },
    ];

    let acc = "";
    try {
      await streamCompletion(payload, model, (delta) => {
        acc += delta;
        setStreaming(acc);
      });
      if (acc.trim()) {
        await db.insert("Message", { conversationId: convId, role: "assistant", content: acc });
      }
    } catch (err) {
      const code = (err as { code?: string })?.code;
      if (code === "AI_NOT_CONFIGURED") {
        setNotice(
          "AI isn't configured yet. Set PYLON_AI_PROVIDER and PYLON_AI_API_KEY in .env, then restart — see the README.",
        );
      } else if (code === "MODEL_OVERRIDE_FORBIDDEN" || code === "MODEL_NOT_ALLOWED") {
        setNotice(
          "That model isn't enabled. Add it to PYLON_AI_MODELS_ALLOWED in .env (comma-separated), then restart.",
        );
      } else if (code === "RATE_LIMITED") {
        setNotice("You've hit the AI rate limit — try again in a little while.");
      } else {
        setNotice("Something went wrong reaching the model. Try again.");
      }
    } finally {
      setStreaming(null);
      setSending(false);
    }
  }

  const empty = messages.length === 0 && streaming === null;

  return (
    <div className="flex h-[calc(100vh-3.5rem)]">
      <Sidebar
        conversations={conversations}
        currentId={currentId}
        onSelect={selectConversation}
        onNew={newChat}
      />
      <div className="flex flex-1 flex-col bg-white">
        <div ref={scrollRef} className="flex-1 overflow-y-auto">
          <div className="mx-auto max-w-3xl px-4 py-6">
            {empty ? (
              <EmptyState onPick={send} />
            ) : (
              <div className="space-y-5">
                {messages.map((m) => (
                  <Bubble key={m.id} role={m.role} content={m.content} />
                ))}
                {streaming !== null ? <Bubble role="assistant" content={streaming || "…"} streaming /> : null}
              </div>
            )}
            {notice ? (
              <div className="mt-5 rounded-lg border border-amber-200 bg-amber-50 px-4 py-3 text-[13px] leading-relaxed text-amber-800">
                {notice}
              </div>
            ) : null}
          </div>
        </div>

        <Composer
          value={input}
          onChange={setInput}
          onSend={() => send(input)}
          disabled={sending}
          placeholder={chat.inputPlaceholder}
          model={model}
          onModelChange={setModel}
        />
      </div>
    </div>
  );
}

function Sidebar({
  conversations,
  currentId,
  onSelect,
  onNew,
}: {
  conversations: ConversationRow[];
  currentId: string | null;
  onSelect: (id: string) => void;
  onNew: () => void;
}) {
  return (
    <aside className="hidden w-64 shrink-0 flex-col border-r border-zinc-200 bg-paper sm:flex">
      <div className="p-3">
        <button
          type="button"
          onClick={onNew}
          className="flex w-full items-center justify-center gap-2 rounded-lg bg-brand px-3 py-2 text-[13.5px] font-medium text-white transition-opacity hover:opacity-90"
        >
          <PlusIcon /> New chat
        </button>
      </div>
      <nav className="flex-1 overflow-y-auto px-2 pb-3">
        {conversations.length === 0 ? (
          <p className="px-2 py-4 text-[12.5px] text-zinc-400">No conversations yet.</p>
        ) : (
          <ul className="space-y-0.5">
            {conversations.map((c) => (
              <li key={c.id}>
                <button
                  type="button"
                  onClick={() => onSelect(c.id)}
                  className={
                    "w-full truncate rounded-lg px-2.5 py-2 text-left text-[13.5px] transition-colors " +
                    (c.id === currentId ? "bg-brand-soft font-medium text-brand" : "text-zinc-600 hover:bg-zinc-100")
                  }
                  title={c.title}
                >
                  {c.title || "New chat"}
                </button>
              </li>
            ))}
          </ul>
        )}
      </nav>
    </aside>
  );
}

function EmptyState({ onPick }: { onPick: (text: string) => void }) {
  const { chat, brand } = siteConfig;
  return (
    <div className="flex flex-col items-center pt-[12vh] text-center">
      <span className="flex size-12 items-center justify-center rounded-2xl bg-brand text-xl font-bold text-white">
        {brand.letter}
      </span>
      <h1 className="mt-5 text-2xl font-semibold tracking-tight text-zinc-900">{chat.emptyHeadline}</h1>
      <p className="mt-2 max-w-md text-[15px] leading-relaxed text-zinc-500">{chat.emptySubcopy}</p>
      <div className="mt-7 grid w-full max-w-xl gap-2 sm:grid-cols-2">
        {chat.suggestions.map((s) => (
          <button
            key={s}
            type="button"
            onClick={() => onPick(s)}
            className="rounded-xl border border-zinc-200 bg-white px-4 py-3 text-left text-[13.5px] text-zinc-600 transition-colors hover:border-brand hover:text-zinc-900"
          >
            {s}
          </button>
        ))}
      </div>
    </div>
  );
}

function Bubble({ role, content, streaming }: { role: string; content: string; streaming?: boolean }) {
  const { brand } = siteConfig;
  const isUser = role === "user";
  return (
    <div className={"flex gap-3 " + (isUser ? "flex-row-reverse" : "")}>
      <span
        className={
          "mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-full text-[11px] font-semibold " +
          (isUser ? "bg-zinc-200 text-zinc-600" : "bg-brand text-white")
        }
      >
        {isUser ? "You" : brand.letter}
      </span>
      <div
        className={
          "max-w-[80%] whitespace-pre-wrap rounded-2xl px-4 py-2.5 text-[14.5px] leading-relaxed " +
          (isUser ? "bg-zinc-900 text-white" : "bg-zinc-100 text-zinc-800")
        }
      >
        {content}
        {streaming ? <span className="ml-0.5 inline-block h-4 w-1.5 animate-pulse bg-zinc-400 align-middle" /> : null}
      </div>
    </div>
  );
}

function Composer({
  value,
  onChange,
  onSend,
  disabled,
  placeholder,
  model,
  onModelChange,
}: {
  value: string;
  onChange: (v: string) => void;
  onSend: () => void;
  disabled: boolean;
  placeholder: string;
  model: string;
  onModelChange: (m: string) => void;
}) {
  const { models } = siteConfig.chat;
  function onKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      onSend();
    }
  }
  return (
    <div className="border-t border-zinc-200 bg-white px-4 py-3">
      <div className="mx-auto flex max-w-3xl items-end gap-2">
        <textarea
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={onKeyDown}
          rows={1}
          placeholder={placeholder}
          aria-label="Message"
          className="max-h-40 flex-1 resize-none rounded-2xl border border-zinc-300 bg-white px-4 py-2.5 text-[14.5px] text-zinc-900 outline-none transition placeholder:text-zinc-400 focus:border-brand focus:ring-2 focus:ring-brand/20"
        />
        <button
          type="button"
          onClick={onSend}
          disabled={disabled || !value.trim()}
          aria-label="Send"
          className="flex size-11 shrink-0 items-center justify-center rounded-2xl bg-brand text-white transition-opacity hover:opacity-90 disabled:opacity-40"
        >
          <SendIcon />
        </button>
      </div>
      <div className="mx-auto mt-1.5 flex max-w-3xl items-center justify-between gap-3">
        <label className="flex items-center gap-1.5 text-[11px] text-zinc-400">
          <span className="hidden sm:inline">Model</span>
          <select
            value={model}
            onChange={(e) => onModelChange(e.target.value)}
            aria-label="Model"
            className="rounded-md border border-zinc-200 bg-white px-1.5 py-1 text-[11.5px] text-zinc-600 outline-none focus:border-brand"
          >
            {models.map((m) => (
              <option key={m.id} value={m.id}>
                {m.label} · {m.provider}
              </option>
            ))}
          </select>
        </label>
        <span className="text-[11px] text-zinc-400">Enter to send · Shift+Enter for a new line</span>
      </div>
    </div>
  );
}

// Parse the OpenAI-style SSE stream from POST /api/ai/stream:
//   data: {"choices":[{"delta":{"content":"…"}}]}   …   data: [DONE]
// Throws { code } on the 503 (AI not configured) / 429 (rate limited) shims.
async function streamCompletion(
  messages: { role: string; content: string }[],
  model: string,
  onDelta: (delta: string) => void,
): Promise<void> {
  const res = await fetch("/api/ai/stream", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ messages, model }),
  });
  if (!res.ok || !res.body) {
    let code = `HTTP_${res.status}`;
    try {
      code = (await res.json())?.error?.code ?? code;
    } catch {
      /* ignore */
    }
    throw { code };
  }
  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buf = "";
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    buf += decoder.decode(value, { stream: true });
    const lines = buf.split("\n");
    buf = lines.pop() ?? "";
    for (const raw of lines) {
      const line = raw.trim();
      if (!line.startsWith("data:")) continue;
      const data = line.slice(5).trim();
      if (data === "[DONE]") return;
      try {
        const j = JSON.parse(data);
        if (j.error) throw { code: j.error.code ?? "STREAM_ERROR" };
        const delta = j.choices?.[0]?.delta?.content;
        if (delta) onDelta(delta);
      } catch (e) {
        if ((e as { code?: string })?.code) throw e;
        /* ignore keep-alive / partial lines */
      }
    }
  }
}

function PlusIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" aria-hidden>
      <path d="M12 5v14M5 12h14" />
    </svg>
  );
}
function SendIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M22 2 11 13M22 2l-7 20-4-9-9-4 20-7z" />
    </svg>
  );
}

