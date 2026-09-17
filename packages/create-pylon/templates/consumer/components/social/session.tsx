"use client";

import React, { useEffect } from "react";
import { callFn } from "@pylonsync/react";
import { EnsureGuest } from "@pylonsync/client";

// Every island on a page mounts its own <SocialGate>, so the first-visit work
// runs once per page load, not once per island: load the demo data, then make
// sure the visitor has a profile.
let bootstrap: Promise<void> | null = null;

function bootstrapOnce(): Promise<void> {
  bootstrap ??= (async () => {
    await callFn("seedFeed", {});
    await callFn("ensureProfile", {});
  })().catch((error) => {
    // Let the next island retry instead of keeping a failed promise.
    bootstrap = null;
    throw error;
  });
  return bootstrap;
}

/**
 * A guest session for anyone who opens the app, so they can like, comment,
 * follow, and post without a sign-up form. Children render as soon as the
 * session exists; live queries fill in as the demo data and profile arrive.
 */
export function SocialGate({ children, fallback = null }: { children: React.ReactNode; fallback?: React.ReactNode }) {
  return (
    <EnsureGuest fallback={fallback}>
      <Bootstrap />
      {children}
    </EnsureGuest>
  );
}

function Bootstrap() {
  useEffect(() => {
    void bootstrapOnce().catch(() => {});
  }, []);
  return null;
}
