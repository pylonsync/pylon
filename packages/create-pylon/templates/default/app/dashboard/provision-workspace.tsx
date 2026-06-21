"use client";

import React, { useEffect, useState } from "react";
import { createOrg, listOrgs } from "@pylonsync/client";
import { db } from "@pylonsync/react";

// Org-less safety net. New signups get "My Workspace" auto-created in the signup
// flow, so this only renders in edge cases: that creation failed, or a user left
// or deleted their last workspace. Pick an existing org if one turned up,
// otherwise create "My Workspace", make it active, and reload into the ready
// dashboard. No multi-step onboarding — there's nothing for the user to decide.
export function ProvisionWorkspace() {
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const orgs = await listOrgs();
        const target = orgs[0] ?? (await createOrg("My Workspace"));
        await db.sync.selectOrg(target.id);
        if (!cancelled) window.location.reload();
      } catch (err) {
        if (!cancelled) {
          setError(
            err instanceof Error
              ? err.message
              : "Couldn't set up your workspace.",
          );
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  async function signOut() {
    await db.sync.signOut();
    window.location.assign("/login");
  }

  return (
    <div className="flex min-h-screen flex-col items-center justify-center gap-4 bg-zinc-50 px-6 text-center">
      {error ? (
        <>
          <p className="max-w-sm text-sm text-red-700">{error}</p>
          <button
            type="button"
            onClick={() => window.location.reload()}
            className="inline-flex h-9 items-center rounded-lg bg-zinc-900 px-4 text-[13px] font-medium text-white transition-colors hover:bg-zinc-700"
          >
            Try again
          </button>
        </>
      ) : (
        <>
          <span
            aria-hidden
            className="size-5 animate-spin rounded-full border-2 border-zinc-300 border-t-zinc-900"
          />
          <p className="text-sm text-zinc-500">Setting up your workspace…</p>
        </>
      )}
      <button
        type="button"
        onClick={signOut}
        className="mt-1 text-[13px] text-zinc-400 underline underline-offset-2 transition-colors hover:text-zinc-600"
      >
        Wrong account? Sign out
      </button>
    </div>
  );
}
