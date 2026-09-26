"use client";

import React, { useEffect, useLayoutEffect, useRef, useState } from "react";
import { siteConfig } from "@/lib/site.config";
import { findModel, modelAllowedFor } from "@/lib/chat";
import { ArrowUpIcon, CheckIcon, ChevronDownIcon, StopIcon } from "./icons";

const useIsoLayoutEffect = typeof window === "undefined" ? useEffect : useLayoutEffect;

export interface ComposerProps {
  value: string;
  onChange: (value: string) => void;
  onSubmit: () => void;
  onStop: () => void;
  // A reply is generating: the send button becomes Stop.
  busy: boolean;
  placeholder: string;
  model: string;
  onModelChange: (id: string) => void;
  // Guests see the models outside `guestModels` as needing an account.
  isGuest?: boolean;
  autoFocus?: boolean;
  textareaRef?: React.RefObject<HTMLTextAreaElement | null>;
}

// The message box. Enter sends, Shift+Enter adds a line, Escape stops a reply.
export function Composer({
  value,
  onChange,
  onSubmit,
  onStop,
  busy,
  placeholder,
  model,
  onModelChange,
  isGuest = false,
  autoFocus,
  textareaRef,
}: ComposerProps) {
  const ownRef = useRef<HTMLTextAreaElement>(null);
  const ref = textareaRef ?? ownRef;
  const canSend = value.trim().length > 0 && !busy;
  const tooLong = value.length > siteConfig.chat.maxInputChars;

  useIsoLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "0px";
    el.style.height = `${Math.min(el.scrollHeight, 240)}px`;
  }, [value, ref]);

  function onKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      if (canSend && !tooLong) onSubmit();
    } else if (e.key === "Escape" && busy) {
      e.preventDefault();
      onStop();
    }
  }

  return (
    <form
      className="bezel composer-shell"
      onSubmit={(e) => {
        e.preventDefault();
        if (canSend && !tooLong) onSubmit();
      }}
    >
      <div className="bezel-core flex flex-col px-3 pb-2.5 pt-3 transition-shadow duration-300">
        <textarea
          ref={ref}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={onKeyDown}
          rows={1}
          autoFocus={autoFocus}
          placeholder={placeholder}
          aria-label="Message"
          className="max-h-60 min-h-[28px] w-full resize-none bg-transparent px-1.5 text-[15.5px] leading-[1.6] text-ink outline-none placeholder:text-ink-3"
        />
        <div className="mt-2 flex items-center justify-between gap-2">
          <ModelPicker value={model} onChange={onModelChange} isGuest={isGuest} />
          <div className="flex items-center gap-3">
            {tooLong ? (
              <span className="text-[12px] text-warn">
                {value.length.toLocaleString()} / {siteConfig.chat.maxInputChars.toLocaleString()}
              </span>
            ) : null}
            {busy ? (
              <button
                type="button"
                onClick={onStop}
                aria-label="Stop"
                title="Stop (Esc)"
                className="press flex size-9 items-center justify-center rounded-full bg-ink text-bg transition-transform duration-200"
              >
                <StopIcon size={11} />
              </button>
            ) : (
              <button
                type="submit"
                disabled={!canSend || tooLong}
                aria-label="Send"
                title="Send (Enter)"
                className="press flex size-9 items-center justify-center rounded-full bg-brand text-white transition-[opacity,transform] duration-200 disabled:bg-ink/15 disabled:text-ink-3"
              >
                <ArrowUpIcon size={18} />
              </button>
            )}
          </div>
        </div>
      </div>
    </form>
  );
}

function ModelPicker({
  value,
  onChange,
  isGuest,
}: {
  value: string;
  onChange: (id: string) => void;
  isGuest: boolean;
}) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const current = findModel(value) ?? siteConfig.chat.models[0];

  useEffect(() => {
    if (!open) return;
    function onDown(e: MouseEvent) {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div ref={rootRef} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="listbox"
        aria-expanded={open}
        className="press inline-flex h-8 items-center gap-1 rounded-lg px-2 text-[13px] font-medium text-ink-2 transition-colors duration-200 hover:bg-hover hover:text-ink"
      >
        {current.label}
        <ChevronDownIcon size={14} className={"transition-transform duration-300 " + (open ? "rotate-180" : "")} />
      </button>
      {open ? (
        <div
          role="listbox"
          aria-label="Model"
          className="menu-in absolute bottom-10 left-0 z-30 w-[17rem] rounded-2xl border border-line bg-surface p-1.5 shadow-float"
        >
          {siteConfig.chat.models.map((m) => {
            const locked = !modelAllowedFor(m.id, isGuest);
            return (
              <button
                key={m.id}
                type="button"
                role="option"
                aria-selected={m.id === current.id}
                aria-disabled={locked}
                disabled={locked}
                onClick={() => {
                  onChange(m.id);
                  setOpen(false);
                }}
                className="flex w-full items-center gap-3 rounded-xl px-3 py-2 text-left transition-colors duration-150 enabled:hover:bg-hover disabled:opacity-55"
              >
                <span className="min-w-0 flex-1">
                  <span className="block text-[13.5px] font-medium text-ink">{m.label}</span>
                  <span className="block text-[12.5px] text-ink-3">{locked ? "Sign in to use this model" : m.note}</span>
                </span>
                {m.id === current.id ? <CheckIcon size={16} className="text-brand" /> : null}
              </button>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}
