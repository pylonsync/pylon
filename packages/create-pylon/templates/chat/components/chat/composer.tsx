"use client";

import React, { useEffect, useRef, useState } from "react";
import { ArrowUp } from "lucide-react";
import { cn } from "@/lib/utils";
import { MAX_MESSAGE_LENGTH } from "@/lib/chat";

interface ComposerProps {
  channelName: string;
  /** Changes when the channel changes; the draft clears and the field refocuses. */
  channelKey: string;
  onSend: (text: string) => void;
  /** Called on each keystroke that leaves text in the field. */
  onTyping?: () => void;
  /** Called when the field is emptied or the message is sent. */
  onStopTyping?: () => void;
  disabled?: boolean;
}

/**
 * Message field. Enter sends, Shift+Enter adds a line, and the field grows
 * up to about eight lines. Enter during IME composition (Japanese, Chinese,
 * Korean input) confirms the composition instead of sending.
 */
export function Composer({
  channelName,
  channelKey,
  onSend,
  onTyping,
  onStopTyping,
  disabled,
}: ComposerProps) {
  const [text, setText] = useState("");
  const ref = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    setText("");
    // Focus on desktop only: on a phone, focusing opens the keyboard over the conversation.
    if (window.matchMedia("(pointer: fine)").matches) ref.current?.focus();
  }, [channelKey]);

  // Grow with the content, capped by max-height in the class list.
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [text]);

  const canSend = text.trim().length > 0 && !disabled;

  function submit() {
    const value = text.trim();
    if (!value || disabled) return;
    onSend(value);
    setText("");
    onStopTyping?.();
    ref.current?.focus();
  }

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
      className="composer group/composer relative flex items-end gap-2 rounded-2xl bg-card p-1.5 pl-4 shadow-composer transition-shadow duration-200 ease-out-soft focus-within:shadow-composer-focus"
    >
      <label htmlFor="composer" className="sr-only">
        Message #{channelName}
      </label>
      <textarea
        id="composer"
        ref={ref}
        rows={1}
        value={text}
        maxLength={MAX_MESSAGE_LENGTH}
        placeholder={`Message #${channelName}`}
        autoComplete="off"
        enterKeyHint="send"
        onChange={(e) => {
          setText(e.target.value);
          if (e.target.value.trim()) onTyping?.();
          else onStopTyping?.();
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
            e.preventDefault();
            submit();
          }
        }}
        onBlur={() => onStopTyping?.()}
        className="max-h-48 min-h-9 flex-1 resize-none bg-transparent py-[7px] text-base leading-[22px] sm:text-[15px] text-foreground outline-none placeholder:text-muted-foreground/80"
      />
      <button
        type="submit"
        disabled={!canSend}
        aria-label="Send message"
        className={cn(
          "inline-flex size-9 shrink-0 items-center justify-center rounded-xl transition-all duration-200 ease-out-soft",
          canSend
            ? "bg-signal text-signal-foreground shadow-sm hover:brightness-105 active:scale-95"
            : "bg-muted text-muted-foreground/60",
        )}
      >
        <ArrowUp className="size-[18px]" strokeWidth={2} aria-hidden />
      </button>
    </form>
  );
}
