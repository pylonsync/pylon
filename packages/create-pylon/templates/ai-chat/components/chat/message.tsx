"use client";

import React, { useState } from "react";
import { describeError, modelLabel } from "@/lib/chat";
import { Markdown } from "./markdown";
import { ProviderSetup } from "./setup";
import { AlertIcon, BrandMark, CheckIcon, CopyIcon, RefreshIcon } from "./icons";

export function UserMessage({ content, pending }: { content: string; pending?: boolean }) {
  return (
    <div className="msg-in flex justify-end">
      <div
        className={
          "max-w-[min(85%,36rem)] whitespace-pre-wrap break-words rounded-[20px] rounded-br-md bg-bubble px-4 py-2.5 text-[15px] leading-relaxed text-ink " +
          (pending ? "opacity-70" : "")
        }
      >
        {content}
      </div>
    </div>
  );
}

export interface AssistantMessageProps {
  content: string;
  // "streaming" | "done" | "stopped" | "error" | "interrupted"
  status: string;
  errorCode?: string | null;
  model?: string | null;
  // Regenerate and Retry are offered on the newest reply only.
  canRegenerate: boolean;
  onRegenerate?: () => void;
  providerConfigured?: boolean;
}

export function AssistantMessage({
  content,
  status,
  errorCode,
  model,
  canRegenerate,
  onRegenerate,
  providerConfigured,
}: AssistantMessageProps) {
  const streaming = status === "streaming";
  const failed = status === "error";

  return (
    <div className="msg-in group/msg flex gap-3.5 sm:gap-4">
      <div className="hidden pt-0.5 sm:block">
        <BrandMark size={26} />
      </div>
      <div className="min-w-0 flex-1">
        {content ? (
          <div className={streaming ? "streaming-caret" : undefined}>
            <Markdown text={content} />
          </div>
        ) : streaming ? (
          <ThinkingDots />
        ) : null}

        {failed ? (
          errorCode === "LLM_NOT_CONFIGURED" && !providerConfigured ? (
            <div className="mt-1">
              <ProviderSetup compact />
            </div>
          ) : (
            <ErrorNote code={errorCode} />
          )
        ) : null}

        {status === "interrupted" ? (
          <p className="mt-2 text-[13px] text-ink-3">This reply was interrupted before it finished.</p>
        ) : null}

        {!streaming ? (
          <MessageActions
            content={content}
            status={status}
            model={model}
            canRegenerate={canRegenerate}
            onRegenerate={onRegenerate}
          />
        ) : null}
      </div>
    </div>
  );
}

function ThinkingDots() {
  return (
    <div className="flex h-7 items-center gap-1.5" role="status" aria-label="Writing a reply">
      <span className="thinking-dot" />
      <span className="thinking-dot [animation-delay:160ms]" />
      <span className="thinking-dot [animation-delay:320ms]" />
    </div>
  );
}

function ErrorNote({ code }: { code?: string | null }) {
  const copy = describeError(code);
  return (
    <div className="mt-2 flex gap-3 rounded-2xl border border-warn/20 bg-warn-soft px-4 py-3">
      <AlertIcon size={18} className="mt-0.5 shrink-0 text-warn" />
      <div>
        <p className="text-[14px] font-medium text-ink">{copy.title}</p>
        <p className="mt-0.5 text-[13.5px] leading-relaxed text-ink-2">{copy.body}</p>
      </div>
    </div>
  );
}

function MessageActions({
  content,
  status,
  model,
  canRegenerate,
  onRegenerate,
}: {
  content: string;
  status: string;
  model?: string | null;
  canRegenerate: boolean;
  onRegenerate?: () => void;
}) {
  const [copied, setCopied] = useState(false);

  async function copy() {
    try {
      await navigator.clipboard.writeText(content);
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    } catch {
      /* clipboard blocked */
    }
  }

  const retry = status === "error" || status === "interrupted";

  return (
    <div className="mt-2 flex h-8 items-center gap-0.5 text-ink-3">
      {content ? (
        <ActionButton label={copied ? "Copied" : "Copy"} onClick={copy}>
          {copied ? <CheckIcon size={16} /> : <CopyIcon size={16} />}
        </ActionButton>
      ) : null}
      {canRegenerate && onRegenerate ? (
        <ActionButton label={retry ? "Retry" : "Regenerate"} onClick={onRegenerate}>
          <RefreshIcon size={16} />
          {retry ? <span className="pr-1 text-[13px]">Retry</span> : null}
        </ActionButton>
      ) : null}
      {status === "stopped" ? <span className="ml-2 text-[12.5px]">Stopped</span> : null}
      {model && status !== "error" ? (
        <span className="ml-2 text-[12.5px] opacity-0 transition-opacity duration-300 group-hover/msg:opacity-100">
          {modelLabel(model)}
        </span>
      ) : null}
    </div>
  );
}

function ActionButton({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      title={label}
      className="press inline-flex h-8 min-w-8 items-center justify-center gap-1 rounded-lg px-1.5 transition-colors duration-200 hover:bg-hover hover:text-ink"
    >
      {children}
    </button>
  );
}
