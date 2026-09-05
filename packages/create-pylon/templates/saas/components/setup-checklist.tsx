"use client";

import React, { useState } from "react";
import { callFn } from "@pylonsync/react";
import { Check, X } from "lucide-react";

export interface SetupState {
  orgId: string;
  hasProject: boolean;
  hasTeammate: boolean;
  hasBilling: boolean;
  canDismiss: boolean;
}

/**
 * "Getting started" on the Overview. Items are derived from real data (a
 * project exists, a teammate or pending invite exists, Pro is active), so the
 * list ticks itself off; it disappears when all three are done or an
 * owner/admin dismisses it for the workspace.
 */
export function SetupChecklist({ state }: { state: SetupState }) {
  const [hidden, setHidden] = useState(false);
  const items = [
    { done: state.hasProject, label: "Create a project", href: "/dashboard/projects" },
    { done: state.hasTeammate, label: "Invite a teammate", href: "/dashboard/members" },
    { done: state.hasBilling, label: "Start your Pro trial", href: "/dashboard/billing" },
  ];
  const remaining = items.filter((i) => !i.done).length;
  if (hidden || remaining === 0) return null;

  async function dismiss() {
    setHidden(true);
    try {
      await callFn("dismissSetup", { orgId: state.orgId });
    } catch {
      // Hidden for this view regardless; the next load re-evaluates.
    }
  }

  return (
    <div className="rounded-xl border border-zinc-200 bg-white p-5">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-sm font-semibold text-zinc-900">Getting started</h2>
          <p className="mt-0.5 text-xs text-zinc-500">
            {items.length - remaining} of {items.length} done
          </p>
        </div>
        {state.canDismiss && (
          <button
            type="button"
            onClick={() => void dismiss()}
            aria-label="Dismiss checklist"
            className="flex size-7 items-center justify-center rounded-md text-zinc-400 hover:bg-zinc-100 hover:text-zinc-700"
          >
            <X className="size-4" />
          </button>
        )}
      </div>
      <ul className="mt-3 divide-y divide-zinc-100">
        {items.map((it) => (
          <li key={it.label} className="flex items-center gap-3 py-2.5">
            <span
              className={
                "flex size-5 shrink-0 items-center justify-center rounded-full border " +
                (it.done ? "border-emerald-500 bg-emerald-500 text-white" : "border-zinc-300")
              }
            >
              {it.done && <Check className="size-3" />}
            </span>
            {it.done ? (
              <span className="text-sm text-zinc-400 line-through">{it.label}</span>
            ) : (
              <a href={it.href} className="text-sm font-medium text-zinc-900 hover:underline">
                {it.label} →
              </a>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}
