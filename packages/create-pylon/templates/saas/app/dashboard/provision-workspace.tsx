"use client";

import React, { useEffect, useState } from "react";
import { listOrgs } from "@pylonsync/client";
import { db } from "@pylonsync/react";

// Org-less safety net. A new account goes through /onboarding, so this only
// renders when a user lost their active workspace (left or deleted their last
// org, or the wizard was abandoned before step 1). Re-select an existing org
// if one turned up; otherwise send them to the wizard to create one.
export function ProvisionWorkspace() {
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const orgs = await listOrgs();
        if (orgs.length === 0) {
          if (!cancelled) window.location.assign("/onboarding");
          return;
        }
        await db.sync.selectOrg(orgs[0].id);
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
