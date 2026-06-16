"use client";

import React, { useEffect } from "react";
import { callFn } from "@pylonsync/react";
import { EnsureGuest } from "@pylonsync/client";

// A zero-UI client island that seeds the public Project portfolio on first
// visit (idempotent server-side — a no-op once any project exists). Drop it on
// any public page so /work and the case-study pages aren't empty even if the
// visitor never hit the homepage. Wrapped in <EnsureGuest> so a session exists
// for the call; seedProjects is a public mutation, so an anonymous guest can run
// it (it only ever writes the config's marketing copy — no PII).
export function SeedProjects() {
  return (
    <EnsureGuest fallback={null}>
      <Seed />
    </EnsureGuest>
  );
}

function Seed() {
  useEffect(() => {
    void callFn("seedProjects", {});
  }, []);
  return null;
}
