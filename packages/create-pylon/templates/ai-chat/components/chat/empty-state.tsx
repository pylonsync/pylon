"use client";

import React from "react";
import { siteConfig } from "@/lib/site.config";
import { ProviderSetup } from "./setup";
import { BookIcon } from "./icons";

export interface EmptyStateProps {
  // The composer, placed under the headline.
  composer: React.ReactNode;
  onPick: (prompt: string) => void;
  providerConfigured: boolean | null;
  exampleCount: number;
  onOpenExample?: () => void;
}

// The new-chat screen: headline, composer, starter prompts, and the setup
// steps while no provider key is configured. Below the md breakpoint the
// composer is docked at the bottom of the screen instead.
export function EmptyState({ composer, onPick, providerConfigured, exampleCount, onOpenExample }: EmptyStateProps) {
  const { chat } = siteConfig;
  return (
    <div className="mx-auto flex w-full max-w-[46rem] flex-col px-4 pb-10 pt-[max(10vh,3rem)] sm:px-6 md:pt-[16vh]">
      <h1 className="rise text-center font-serif text-[30px] font-normal leading-tight tracking-[-0.02em] text-ink sm:text-[38px]">
        {chat.emptyHeadline}
      </h1>

      <div className="rise mt-9 hidden [animation-delay:60ms] md:block">{composer}</div>

      <div className="rise mt-4 grid grid-cols-2 gap-2 [animation-delay:120ms] max-md:mt-8">
        {chat.suggestions.map((s) => (
          <button
            key={s.title}
            type="button"
            onClick={() => onPick(s.prompt)}
            className="press group/s rounded-2xl px-4 py-3 text-left ring-1 ring-line transition-[background-color,box-shadow] duration-300 hover:bg-surface hover:shadow-soft"
          >
            <span className="block text-[14px] font-medium text-ink">{s.title}</span>
            <span className="mt-0.5 line-clamp-2 text-[13px] text-ink-3 sm:line-clamp-1 transition-colors duration-300 group-hover/s:text-ink-2">
              {s.prompt}
            </span>
          </button>
        ))}
      </div>

      {providerConfigured === false ? (
        <div className="rise mt-8 space-y-3 [animation-delay:180ms]">
          <ProviderSetup />
          {exampleCount > 0 && onOpenExample ? (
            <button
              type="button"
              onClick={onOpenExample}
              className="press flex w-full items-center gap-3 rounded-2xl px-4 py-3 text-left ring-1 ring-line transition-colors duration-300 hover:bg-surface"
            >
              <BookIcon size={18} className="shrink-0 text-ink-3" />
              <span className="flex-1 text-[13.5px] text-ink-2">
                Open an example conversation to see how replies are laid out.
              </span>
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
