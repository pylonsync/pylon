"use client";

import React, { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { ArrowDown, Trash2 } from "lucide-react";
import { cn } from "@/lib/utils";
import {
  dayLabel,
  groupMessages,
  timeLabel,
  type ChatMember,
  type ChatMessage,
} from "@/lib/chat";
import { Avatar } from "./avatar";

interface MessageListProps {
  channelName: string;
  channelTopic: string;
  /** Sorted oldest first. */
  messages: ChatMessage[];
  members: Map<string, ChatMember>;
  selfId: string;
  onDelete: (id: string) => void;
}

/** Distance from the bottom, in px, that still counts as "at the bottom". */
const STICK_THRESHOLD = 96;

/**
 * The conversation for one channel. Render it with `key={channelId}` so a
 * channel switch starts at the bottom with fresh scroll state. Keeps the newest message in view while the reader is at
 * the bottom, and always after they send. When they have scrolled up to read
 * history, new messages from others show a "New messages" button instead of
 * yanking the scroll position.
 */
export function MessageList({
  channelName,
  channelTopic,
  messages,
  members,
  selfId,
  onDelete,
}: MessageListProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const atBottomRef = useRef(true);
  const lastCountRef = useRef(0);
  const lastIdRef = useRef<string | null>(null);
  // Rows already on screen when the list first filled. They render without
  // the entrance animation; rows that arrive later get it.
  const [knownIds] = useState(() => new Set(messages.map((m) => m.id)));
  const [unseen, setUnseen] = useState(0);

  const groups = useMemo(() => groupMessages(messages), [messages]);
  // No entrance animation while the list is still empty: the first rows to
  // arrive are history, not new messages.
  const animateNew = lastCountRef.current > 0;

  // Follow new rows when the reader is at the bottom or sent the row. The
  // first fill (on mount, or when the replica loads) jumps without animating.
  useLayoutEffect(() => {
    const el = scrollRef.current;
    const previous = lastCountRef.current;
    const added = messages.length - previous;
    lastCountRef.current = messages.length;
    if (!el || added <= 0) return;
    if (previous === 0) {
      lastIdRef.current = messages[messages.length - 1]?.id ?? null;
      for (const m of messages) knownIds.add(m.id);
      el.scrollTop = el.scrollHeight;
      return;
    }
    const last = messages[messages.length - 1];
    const previousLastId = lastIdRef.current;
    lastIdRef.current = last?.id ?? null;
    // Rows that sorted in above the newest one are history catching up, not
    // new messages: keep the view pinned if it was, and count nothing.
    if (last?.id === previousLastId) {
      if (atBottomRef.current) el.scrollTop = el.scrollHeight;
      return;
    }
    // An instant pin: a smooth scroll can emit a mid-animation scroll event
    // that reads as "not at the bottom" when the next row lands.
    if (atBottomRef.current || last?.authorId === selfId) {
      el.scrollTop = el.scrollHeight;
      atBottomRef.current = true;
      setUnseen(0);
    } else {
      const fromOthers = messages.slice(-added).filter((m) => m.authorId !== selfId).length;
      if (fromOthers > 0) setUnseen((n) => n + fromOthers);
    }
  }, [messages, selfId, knownIds]);

  // Stay pinned to the newest message when the viewport shrinks (the phone
  // keyboard opens) or the composer grows.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      if (atBottomRef.current) el.scrollTop = el.scrollHeight;
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  function onScroll() {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < STICK_THRESHOLD;
    atBottomRef.current = atBottom;
    if (atBottom && unseen) setUnseen(0);
  }

  function jumpToLatest() {
    const el = scrollRef.current;
    el?.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
    setUnseen(0);
  }

  let previousDay: string | null = null;

  return (
    <div className="relative min-h-0 flex-1">
      <div
        ref={scrollRef}
        onScroll={onScroll}
        role="log"
        aria-live="polite"
        aria-label={`Messages in #${channelName}`}
        className="scroll-area h-full overflow-y-auto overscroll-contain"
      >
        <div className="flex min-h-full max-w-4xl flex-col justify-end px-3 pt-10 pb-4 sm:px-5">
          <ChannelIntro name={channelName} topic={channelTopic} empty={messages.length === 0} />

          {groups.map((group) => {
            const first = group.messages[0];
            const showDay = group.day !== null && group.day !== previousDay;
            if (group.day !== null) previousDay = group.day;
            const author = members.get(group.authorId);
            const name = author?.name ?? "Someone";
            const hue = author?.hue ?? 250;
            const isSelf = group.authorId === selfId;
            return (
              <React.Fragment key={group.key}>
                {showDay && first.createdAt ? <DayDivider iso={first.createdAt} /> : null}
                <section className="group/msggroup mt-3 first:mt-0" aria-label={`${name}`}>
                  {group.messages.map((m, i) => (
                    <MessageRow
                      key={m.id}
                      message={m}
                      head={i === 0}
                      name={name}
                      hue={hue}
                      isSelf={isSelf}
                      animate={animateNew && !knownIds.has(m.id)}
                      onDelete={onDelete}
                    />
                  ))}
                </section>
              </React.Fragment>
            );
          })}
        </div>
      </div>

      <button
        type="button"
        onClick={jumpToLatest}
        aria-hidden={unseen === 0}
        tabIndex={unseen === 0 ? -1 : 0}
        className={cn(
          "absolute bottom-3 left-1/2 inline-flex -translate-x-1/2 items-center gap-1.5 rounded-full bg-foreground px-3.5 py-1.5 text-[13px] font-medium text-background shadow-float transition-all duration-300 ease-out-soft",
          unseen > 0 ? "translate-y-0 opacity-100" : "pointer-events-none translate-y-2 opacity-0",
        )}
      >
        <ArrowDown className="size-3.5" strokeWidth={2} aria-hidden />
        {unseen === 1 ? "1 new message" : `${unseen} new messages`}
      </button>
    </div>
  );
}

function ChannelIntro({ name, topic, empty }: { name: string; topic: string; empty: boolean }) {
  return (
    <div className="mb-8 px-2 sm:px-3">
      <div className="mb-3 inline-flex size-11 items-center justify-center rounded-xl bg-muted text-xl font-medium text-muted-foreground">
        #
      </div>
      <h2 className="text-[22px] font-semibold tracking-tight text-foreground">
        This is the start of #{name}
      </h2>
      <p className="mt-1 text-[15px] text-muted-foreground">
        {empty ? `${topic}. Send the first message.` : topic}
      </p>
    </div>
  );
}

function DayDivider({ iso }: { iso: string }) {
  return (
    <div className="relative mt-6 mb-2 flex items-center justify-center" role="separator">
      <span aria-hidden className="absolute inset-x-2 top-1/2 h-px bg-border sm:inset-x-3" />
      <span className="relative rounded-full bg-background px-3 py-0.5 text-[12px] font-medium text-muted-foreground ring-1 ring-border">
        {dayLabel(iso)}
      </span>
    </div>
  );
}

interface MessageRowProps {
  message: ChatMessage;
  head: boolean;
  name: string;
  hue: number;
  isSelf: boolean;
  animate: boolean;
  onDelete: (id: string) => void;
}

function MessageRow({ message, head, name, hue, isSelf, animate, onDelete }: MessageRowProps) {
  const pending = !message.createdAt;
  return (
    <div
      className={cn(
        "group/msg relative flex gap-3 rounded-lg px-2 py-0.5 transition-colors duration-150 hover:bg-hover sm:px-3",
        head ? "pt-1.5" : "",
        animate && "animate-message-in",
      )}
    >
      <div className="w-9 shrink-0">
        {head ? (
          <Avatar name={name} hue={hue} className="mt-0.5" />
        ) : message.createdAt ? (
          <time
            dateTime={message.createdAt}
            className="block pt-[3px] text-right font-mono text-[10.5px] leading-[22px] text-muted-foreground opacity-0 transition-opacity duration-150 group-hover/msg:opacity-100"
          >
            {timeLabel(message.createdAt).replace(/\s?[AP]M$/i, "")}
          </time>
        ) : null}
      </div>
      <div className="min-w-0 flex-1">
        {head ? (
          <div className="flex items-baseline gap-2">
            <span className="truncate text-[15px] font-semibold text-foreground">{name}</span>
            {isSelf ? (
              <span className="shrink-0 text-[12px] text-muted-foreground">you</span>
            ) : null}
            {message.createdAt ? (
              <time
                dateTime={message.createdAt}
                title={new Date(message.createdAt).toLocaleString()}
                className="shrink-0 font-mono text-[11px] text-muted-foreground"
              >
                {timeLabel(message.createdAt)}
              </time>
            ) : null}
          </div>
        ) : null}
        <p
          className={cn(
            "text-[15px] leading-[22px] whitespace-pre-wrap text-foreground/90 [overflow-wrap:anywhere] transition-opacity duration-200",
            pending && "opacity-55",
          )}
        >
          {message.text}
          {pending ? (
            <span className="ml-2 align-middle font-mono text-[10.5px] text-muted-foreground">
              Sending
            </span>
          ) : null}
        </p>
      </div>
      {isSelf && !pending ? (
        <button
          type="button"
          onClick={() => onDelete(message.id)}
          aria-label="Delete message"
          title="Delete message"
          className="delete-action absolute top-1 right-2 inline-flex size-7 items-center justify-center rounded-md bg-card text-muted-foreground shadow-sm ring-1 ring-border transition-opacity duration-150 hover:text-destructive"
        >
          <Trash2 className="size-3.5" strokeWidth={1.75} aria-hidden />
        </button>
      ) : null}
    </div>
  );
}
