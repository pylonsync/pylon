"use client";

import React, { useState } from "react";
import { callFn, Link } from "@pylonsync/react";
import { Check, ChevronRight, X } from "lucide-react";

export interface SetupState {
  orgId: string;
  hasProject: boolean;
  hasTeammate: boolean;
  hasBilling: boolean;
  canDismiss: boolean;
}

/**
 * "Get started" steps on the Overview. Each step is derived from real data (a
 * project exists, a teammate or pending invite exists, Pro is active), so the
 * list ticks itself off. It disappears when all three are done or an
 * owner/admin dismisses it for the workspace.
 */
export function SetupChecklist({ state }: { state: SetupState }) {
  const [hidden, setHidden] = useState(false);
  const items = [
    { done: state.hasProject, label: "Create a project", href: "/dashboard/projects" },
    { done: state.hasTeammate, label: "Invite a teammate", href: "/dashboard/members" },
    { done: state.hasBilling, label: "Start the Pro trial", href: "/dashboard/billing" },
  ];
  const doneCount = items.filter((i) => i.done).length;
  if (hidden || doneCount === items.length) return null;

  async function dismiss() {
    setHidden(true);
    try {
      await callFn("dismissSetup", { orgId: state.orgId });
    } catch {
      // Hidden for this view regardless; the next load re-evaluates.
    }
  }

  return (
    <div className="rounded-xl border border-zinc-200/80 bg-white">
      <div className="flex items-center justify-between gap-3 border-b border-zinc-100 px-4 py-3">
        <div className="flex items-center gap-3">
          <h2 className="text-[13px] font-semibold text-zinc-900">Get started</h2>
          <span className="font-mono text-[11.5px] tabular-nums text-zinc-400">
            {doneCount}/{items.length}
          </span>
        </div>
        {state.canDismiss && (
          <button
            type="button"
            onClick={() => void dismiss()}
            aria-label="Hide the setup steps"
            className="flex size-7 items-center justify-center rounded-md text-zinc-400 hover:bg-zinc-100 hover:text-zinc-700"
          >
            <X className="size-4" />
          </button>
        )}
      </div>
      <ul className="grid divide-y divide-zinc-100 sm:grid-cols-3 sm:divide-x sm:divide-y-0">
        {items.map((it) => (
          <li key={it.label}>
            {it.done ? (
              <div className="flex items-center gap-2.5 px-4 py-3 text-[13px] text-zinc-400">
                <span className="flex size-[18px] items-center justify-center rounded-full bg-brand text-white">
                  <Check className="size-3" strokeWidth={2.5} />
                </span>
                <span className="line-through">{it.label}</span>
              </div>
            ) : (
              <Link
                href={it.href}
                className="group flex items-center gap-2.5 px-4 py-3 text-[13px] font-medium text-zinc-800 transition-colors hover:bg-zinc-50"
              >
                <span className="size-[18px] rounded-full border-[1.5px] border-dashed border-zinc-300" />
                {it.label}
                <ChevronRight className="ml-auto size-3.5 text-zinc-300 transition-transform group-hover:translate-x-0.5 group-hover:text-zinc-500" />
              </Link>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}
