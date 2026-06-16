"use client";

import React, { useEffect, useRef, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { siteConfig } from "@/lib/site.config";

// The chat app — a client island, rendered only for a SIGNED-IN user (the page
// redirects anyone else to /login). Conversations + messages are sync-backed
// owner-scoped entities (`db.useQuery`), private to your account and in lockstep
// across your tabs + devices. Sending streams tokens from the built-in
// `POST /api/ai/stream` (SSE) — your PYLON_AI_API_KEY never reaches the browser.
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

  useEffect(() => {
    if (!initialized.current && conversations.length > 0) {
      initialized.current = true;
      setCurrentId(conversations[0].id);
    }
  }, [conversations]);

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
  async function renameConversation(id: string, title: string) {
    const t = title.trim().slice(0, 80);
    if (!t) return;
    await db.update("Conversation", id, { title: t });
  }
  async function deleteConversation(id: string) {
    if (currentId === id) setCurrentId(null);
    try {
      await callFn("deleteConversation", { conversationId: id });
    } catch {
      /* ignore */
    }
  }

  async function send(text: string) {
    const trimmed = text.trim();
    if (!trimmed || sending) return;
    setSending(true);
    setNotice(null);
    setInput("");

    const history = messages.map((m) => ({ role: m.role, content: m.content }));
    let convId = currentId;
    if (!convId) {
      convId = await db.insert("Conversation", { title: trimmed.slice(0, 48) });
      setCurrentId(convId);
    }
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
        onRename={renameConversation}
        onDelete={deleteConversation}
      />
      <div className="flex flex-1 flex-col bg-white">
        <div ref={scrollRef} className="flex-1 overflow-y-auto">
          <div className="mx-auto max-w-3xl px-4 py-6">
            {empty ? (
              <EmptyState onPick={send} />
            ) : (
              <div className="space-y-5">
                {messages.map((m) => (
                  <Bubble key={m.id} role={m.role} content={m.content} at={m.createdAt} />
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
  onRename,
  onDelete,
}: {
  conversations: ConversationRow[];
  currentId: string | null;
  onSelect: (id: string) => void;
  onNew: () => void;
  onRename: (id: string, title: string) => void;
  onDelete: (id: string) => void;
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
              <ConversationRow
                key={c.id}
                convo={c}
                active={c.id === currentId}
                onSelect={() => onSelect(c.id)}
                onRename={(t) => onRename(c.id, t)}
                onDelete={() => onDelete(c.id)}
              />
            ))}
          </ul>
        )}
      </nav>
    </aside>
  );
}

function ConversationRow({
  convo,
  active,
  onSelect,
  onRename,
  onDelete,
}: {
  convo: ConversationRow;
  active: boolean;
  onSelect: () => void;
  onRename: (title: string) => void;
  onDelete: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(convo.title);
  const [confirming, setConfirming] = useState(false);

  function commit() {
    setEditing(false);
    if (draft.trim() && draft.trim() !== convo.title) onRename(draft);
    else setDraft(convo.title);
  }

  if (editing) {
    return (
      <li>
        <input
          autoFocus
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit();
            if (e.key === "Escape") {
              setDraft(convo.title);
              setEditing(false);
            }
          }}
          className="w-full rounded-lg border border-brand bg-white px-2.5 py-2 text-[13.5px] outline-none ring-2 ring-brand/20"
        />
      </li>
    );
  }

  return (
    <li className="group relative">
      <button
        type="button"
        onClick={onSelect}
        className={
          "flex w-full items-center rounded-lg py-2 pl-2.5 pr-14 text-left text-[13.5px] transition-colors " +
          (active ? "bg-brand-soft font-medium text-brand" : "text-zinc-600 hover:bg-zinc-100")
        }
        title={convo.title}
      >
        <span className="truncate">{convo.title || "New chat"}</span>
      </button>
      {/* hover actions */}
      <div className="absolute right-1.5 top-1/2 hidden -translate-y-1/2 items-center gap-0.5 group-hover:flex">
        {confirming ? (
          <button
            type="button"
            onClick={onDelete}
            className="rounded px-1.5 py-0.5 text-[11px] font-medium text-red-600 hover:bg-red-50"
          >
            Sure?
          </button>
        ) : (
          <>
            <button
              type="button"
              onClick={() => {
                setDraft(convo.title);
                setEditing(true);
              }}
              aria-label="Rename"
              className="rounded p-1 text-zinc-400 hover:bg-zinc-200 hover:text-zinc-700"
            >
              <PencilIcon />
            </button>
            <button
              type="button"
              onClick={() => {
                setConfirming(true);
                setTimeout(() => setConfirming(false), 2500);
              }}
              aria-label="Delete"
              className="rounded p-1 text-zinc-400 hover:bg-zinc-200 hover:text-red-600"
            >
              <TrashIcon />
            </button>
          </>
        )}
      </div>
    </li>
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

function Bubble({
  role,
  content,
  streaming,
  at,
}: {
  role: string;
  content: string;
  streaming?: boolean;
  at?: string;
}) {
  const { brand } = siteConfig;
  const isUser = role === "user";
  const [copied, setCopied] = useState(false);

  async function copy() {
    try {
      await navigator.clipboard.writeText(content);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      /* clipboard unavailable */
    }
  }

  return (
    <div className={"group flex gap-3 " + (isUser ? "flex-row-reverse" : "")}>
      <span
        className={
          "mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-full text-[11px] font-semibold " +
          (isUser ? "bg-zinc-200 text-zinc-600" : "bg-brand text-white")
        }
      >
        {isUser ? "You" : brand.letter}
      </span>
      <div className={"flex max-w-[80%] flex-col " + (isUser ? "items-end" : "items-start")}>
        <div
          className={
            "rounded-2xl px-4 py-2.5 text-[14.5px] leading-relaxed " +
            (isUser ? "whitespace-pre-wrap bg-zinc-900 text-white" : "bg-zinc-100 text-zinc-800")
          }
        >
          {isUser || streaming ? (
            <span className="whitespace-pre-wrap">{content}</span>
          ) : (
            <Markdown text={content} />
          )}
          {streaming ? <span className="ml-0.5 inline-block h-4 w-1.5 animate-pulse bg-zinc-400 align-middle" /> : null}
        </div>
        {!streaming ? (
          <div className={"mt-1 flex items-center gap-2 px-1 " + (isUser ? "flex-row-reverse" : "")}>
            {at ? <span className="text-[10.5px] text-zinc-300">{relativeTime(at)}</span> : null}
            {!isUser ? (
              <button
                type="button"
                onClick={copy}
                className="text-[10.5px] text-zinc-300 opacity-0 transition-opacity hover:text-zinc-600 group-hover:opacity-100"
              >
                {copied ? "Copied" : "Copy"}
              </button>
            ) : null}
          </div>
        ) : null}
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

/* --------------------------- markdown rendering --------------------------- */
// A small, dependency-free, XSS-safe Markdown renderer for assistant replies. It
// builds React elements directly — raw HTML is never injected into the DOM, so
// model output can't smuggle scripts. Covers what LLMs emit most: fenced code
// blocks, inline `code`, **bold**, *italic*, links, bullet + numbered lists,
// headings, and paragraphs. Swap in `react-markdown` for full CommonMark/GFM.
function Markdown({ text }: { text: string }) {
  return <div className="space-y-2">{renderBlocks(text)}</div>;
}

function renderBlocks(text: string): React.ReactNode[] {
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  const out: React.ReactNode[] = [];
  let i = 0;
  let key = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.trimStart().startsWith("```")) {
      const lang = line.trim().slice(3).trim();
      const code: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trimStart().startsWith("```")) code.push(lines[i++]);
      i++; // closing fence
      out.push(<CodeBlock key={key++} code={code.join("\n")} lang={lang} />);
      continue;
    }
    const h = line.match(/^(#{1,3})\s+(.*)$/);
    if (h) {
      const size = h[1].length === 1 ? "text-[17px]" : h[1].length === 2 ? "text-[15.5px]" : "text-[14.5px]";
      out.push(
        <p key={key++} className={`font-semibold text-zinc-900 ${size}`}>
          {renderInline(h[2])}
        </p>,
      );
      i++;
      continue;
    }
    if (/^\s*[-*]\s+/.test(line)) {
      const items: string[] = [];
      while (i < lines.length && /^\s*[-*]\s+/.test(lines[i])) items.push(lines[i++].replace(/^\s*[-*]\s+/, ""));
      out.push(
        <ul key={key++} className="ml-4 list-disc space-y-1">
          {items.map((it, j) => (
            <li key={j}>{renderInline(it)}</li>
          ))}
        </ul>,
      );
      continue;
    }
    if (/^\s*\d+\.\s+/.test(line)) {
      const items: string[] = [];
      while (i < lines.length && /^\s*\d+\.\s+/.test(lines[i])) items.push(lines[i++].replace(/^\s*\d+\.\s+/, ""));
      out.push(
        <ol key={key++} className="ml-4 list-decimal space-y-1">
          {items.map((it, j) => (
            <li key={j}>{renderInline(it)}</li>
          ))}
        </ol>,
      );
      continue;
    }
    if (line.trim() === "") {
      i++;
      continue;
    }
    const para: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() !== "" &&
      !lines[i].trimStart().startsWith("```") &&
      !/^\s*[-*]\s+/.test(lines[i]) &&
      !/^\s*\d+\.\s+/.test(lines[i]) &&
      !/^#{1,3}\s+/.test(lines[i])
    ) {
      para.push(lines[i++]);
    }
    out.push(
      <p key={key++} className="leading-relaxed">
        {para.flatMap((ln, j) => (j === 0 ? renderInline(ln) : [<br key={`b${j}`} />, ...renderInline(ln)]))}
      </p>,
    );
  }
  return out;
}

function renderInline(text: string): React.ReactNode[] {
  const nodes: React.ReactNode[] = [];
  const re = /(`[^`]+`|\*\*[^*]+\*\*|\*[^*]+\*|\[[^\]]+\]\([^)]+\))/g;
  let last = 0;
  let key = 0;
  for (const m of text.matchAll(re)) {
    const idx = m.index ?? 0;
    if (idx > last) nodes.push(text.slice(last, idx));
    const tok = m[0];
    if (tok.startsWith("`")) {
      nodes.push(
        <code key={key++} className="rounded bg-zinc-200/70 px-1 py-0.5 font-mono text-[12.5px]">
          {tok.slice(1, -1)}
        </code>,
      );
    } else if (tok.startsWith("**")) {
      nodes.push(<strong key={key++}>{tok.slice(2, -2)}</strong>);
    } else if (tok.startsWith("*")) {
      nodes.push(<em key={key++}>{tok.slice(1, -1)}</em>);
    } else {
      const lm = tok.match(/\[([^\]]+)\]\(([^)]+)\)/);
      if (lm) {
        nodes.push(
          <a key={key++} href={lm[2]} target="_blank" rel="noopener noreferrer" className="text-brand underline">
            {lm[1]}
          </a>,
        );
      }
    }
    last = idx + tok.length;
  }
  if (last < text.length) nodes.push(text.slice(last));
  return nodes;
}

function CodeBlock({ code, lang }: { code: string; lang?: string }) {
  const [copied, setCopied] = useState(false);
  async function copy() {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      /* ignore */
    }
  }
  return (
    <div className="overflow-hidden rounded-lg border border-zinc-200 bg-zinc-950">
      <div className="flex items-center justify-between border-b border-white/10 px-3 py-1.5">
        <span className="font-mono text-[10.5px] uppercase tracking-wide text-zinc-400">{lang || "code"}</span>
        <button type="button" onClick={copy} className="text-[10.5px] text-zinc-400 hover:text-white">
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
      <pre className="overflow-x-auto p-3 text-[12.5px] leading-relaxed text-zinc-100">
        <code className="font-mono">{code}</code>
      </pre>
    </div>
  );
}

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

function relativeTime(iso: string): string {
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return "";
  const s = Math.max(0, Math.floor((Date.now() - t) / 1000));
  if (s < 45) return "just now";
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
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
function PencilIcon() {
  return (
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M12 20h9M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z" />
    </svg>
  );
}
function TrashIcon() {
  return (
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M3 6h18M8 6V4h8v2M19 6l-1 14H6L5 6" />
    </svg>
  );
}
