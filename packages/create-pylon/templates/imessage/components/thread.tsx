import React, { useEffect, useLayoutEffect, useRef, useState } from "react";
import { ArrowUp, ChevronLeft, PanelRight } from "lucide-react";
import { Avatar } from "@/components/avatar";
import { buildThread, type ThreadMessage } from "@/lib/format";
import { formatHandle } from "@/lib/handles";
import { cn } from "@/lib/utils";

export interface ThreadContact {
  name: string | null;
  handle: string;
  status: "allowed" | "blocked" | "unknown";
  optedOut: boolean;
}

export interface Notice {
  tone: "warn" | "muted";
  title: string;
  body: React.ReactNode;
}

export function Thread({
  contact,
  messages,
  thinking,
  notice,
  demo,
  onBack,
  onToggleDetails,
  onSend,
}: {
  contact: ThreadContact;
  messages: ThreadMessage[];
  thinking: boolean;
  notice: Notice | null;
  demo: boolean;
  onBack?: () => void;
  onToggleDetails?: () => void;
  onSend: (text: string) => Promise<void>;
}) {
  const items = buildThread(messages);
  const scroller = useRef<HTMLDivElement>(null);
  // Bubbles present on first render do not animate; new ones do.
  const seen = useRef<Set<string> | null>(null);
  if (seen.current === null) seen.current = new Set(messages.map((m) => m.id));

  useLayoutEffect(() => {
    const el = scroller.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages.length, thinking]);

  useEffect(() => {
    for (const m of messages) seen.current?.add(m.id);
  });

  const title = contact.name ?? formatHandle(contact.handle);

  return (
    <section className="flex h-full min-h-0 flex-col bg-background" aria-label={`Conversation with ${title}`}>
      <header className="flex items-center gap-3 border-b px-4 py-2.5 md:px-6">
        {onBack ? (
          <button
            type="button"
            onClick={onBack}
            className="-ml-2 flex items-center rounded-full p-1.5 text-bubble-out md:hidden"
            aria-label="Back to conversations"
          >
            <ChevronLeft className="size-6" strokeWidth={1.75} />
          </button>
        ) : null}
        <Avatar name={contact.name} handle={contact.handle} size={34} />
        <div className="min-w-0 flex-1">
          <h1 className="truncate text-[15px] font-semibold tracking-[-0.01em]">{title}</h1>
          <p className="truncate text-[12px] text-muted-foreground">
            {contact.name ? formatHandle(contact.handle) : "iMessage"}
            {contact.status === "allowed" ? " · Assistant replies" : contact.status === "blocked" ? " · Blocked" : " · Not on allowlist"}
            {demo ? " · Sample conversation" : ""}
          </p>
        </div>
        {onToggleDetails ? (
          <button
            type="button"
            onClick={onToggleDetails}
            className="rounded-full p-2 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground xl:hidden"
            aria-label="Contact details"
          >
            <PanelRight className="size-[18px]" strokeWidth={1.5} />
          </button>
        ) : null}
      </header>

      {notice ? (
        <div
          role="status"
          className={cn(
            "mx-4 mt-3 rounded-xl px-3.5 py-2.5 text-[13px] leading-snug md:mx-6",
            notice.tone === "warn" ? "bg-warn/10 text-foreground ring-1 ring-warn/25" : "bg-muted text-foreground",
          )}
        >
          <p className="font-semibold">{notice.title}</p>
          <div className="mt-0.5 text-muted-foreground">{notice.body}</div>
        </div>
      ) : null}

      <div ref={scroller} className="min-h-0 flex-1 overflow-y-auto px-4 pb-4 pt-2 md:px-6">
        <ol className="mx-auto flex max-w-[720px] flex-col">
          {items.map(({ message, separator, tail, caption, captionTone }) => {
            const out = message.direction === "out";
            const fresh = !seen.current?.has(message.id);
            return (
              <li key={message.id} className="flex flex-col">
                {separator ? (
                  <p className="mb-2 mt-4 text-center text-[11px] font-medium text-muted-foreground">{separator}</p>
                ) : null}
                <div className={cn("flex", out ? "justify-end pl-12" : "justify-start pr-12", tail ? "mb-1.5" : "mb-[3px]")}>
                  <p
                    className={cn(
                      "bubble max-w-[min(34rem,78%)] text-[15px]",
                      out ? "bubble-out bg-bubble-out text-bubble-out-text" : "bubble-in bg-bubble-in text-bubble-in-text",
                      tail && "bubble-tail",
                      message.kind === "owner" && "bg-[#34c759]",
                      message.status === "failed" && out && "opacity-60",
                      fresh && "bubble-enter",
                    )}
                    style={message.kind === "owner" && tail ? ({ "--bubble-out": "#34c759" } as React.CSSProperties) : undefined}
                  >
                    {message.text}
                  </p>
                </div>
                {caption ? (
                  <p
                    className={cn(
                      "-mt-0.5 mb-2 max-w-[80%] text-[11px] leading-snug",
                      out ? "self-end text-right" : "self-start",
                      captionTone === "error" ? "text-destructive" : captionTone === "warn" ? "text-warn" : "text-muted-foreground",
                    )}
                  >
                    {caption}
                  </p>
                ) : null}
              </li>
            );
          })}
          {thinking ? (
            <li className="mb-1.5 flex justify-end pl-12" aria-label="The assistant is writing a reply">
              <span className="bubble bubble-out bubble-tail flex items-center gap-1 bg-bubble-out px-3.5 py-3">
                {[0, 1, 2].map((i) => (
                  <span
                    key={i}
                    className="typing-dot size-[7px] rounded-full bg-white"
                    style={{ animationDelay: `${i * 150}ms` }}
                  />
                ))}
              </span>
            </li>
          ) : null}
        </ol>
      </div>

      <Composer
        name={contact.name?.split(" ")[0] ?? formatHandle(contact.handle)}
        disabled={contact.optedOut}
        onSend={onSend}
      />
    </section>
  );
}

function Composer({
  name,
  disabled,
  onSend,
}: {
  name: string;
  disabled: boolean;
  onSend: (text: string) => Promise<void>;
}) {
  const [text, setText] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(e?: React.FormEvent) {
    e?.preventDefault();
    const value = text.trim();
    if (!value || pending) return;
    setPending(true);
    setError(null);
    try {
      await onSend(value);
      setText("");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not send");
    } finally {
      setPending(false);
    }
  }

  return (
    <form onSubmit={submit} className="border-t bg-background px-4 pb-4 pt-3 md:px-6">
      <div className="mx-auto flex max-w-[720px] items-end gap-2 rounded-[1.3rem] border bg-background py-1 pl-4 pr-1 shadow-[0_1px_2px_rgba(0,0,0,0.04)] transition-shadow focus-within:shadow-[0_0_0_4px_rgba(10,132,255,0.12)]">
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              void submit();
            }
          }}
          rows={1}
          disabled={disabled}
          placeholder={disabled ? "This person texted STOP" : `Text ${name} yourself`}
          aria-label="Message"
          className="max-h-32 min-h-[30px] flex-1 resize-none bg-transparent py-1.5 text-[15px] leading-snug outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed"
        />
        <button
          type="submit"
          disabled={disabled || pending || text.trim() === ""}
          aria-label="Send"
          className="mb-0.5 flex size-[30px] shrink-0 items-center justify-center rounded-full bg-[#34c759] text-white transition-[transform,opacity] duration-300 ease-[cubic-bezier(0.32,0.72,0,1)] active:scale-95 disabled:opacity-30"
        >
          <ArrowUp className="size-4" strokeWidth={2.25} />
        </button>
      </div>
      <p className={cn("mx-auto mt-1.5 max-w-[720px] px-1 text-[11px]", error ? "block text-destructive" : "hidden text-muted-foreground md:block")}>
        {error ?? "Messages you send here go out from the assistant's number, shown in green."}
      </p>
    </form>
  );
}
