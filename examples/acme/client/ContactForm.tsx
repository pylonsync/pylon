"use client";

import React, { useEffect, useRef, useState } from "react";
import { callFn, configureClient, init, storageKey } from "@pylonsync/react";
import { Button } from "@pylonsync/example-ui/button";
import { Input } from "@pylonsync/example-ui/input";
import { cn } from "@pylonsync/example-ui/utils";

// Bootstrap a guest session so callFn can authenticate.
// Idempotent: skips the /api/auth/guest call when a token already exists.
async function ensureGuest(): Promise<void> {
  init({ appName: "acme" });
  configureClient({ appName: "acme" });
  if (window.localStorage.getItem(storageKey("token"))) return;
  try {
    const res = await fetch("/api/auth/guest", { method: "POST" });
    if (!res.ok) return;
    const body = (await res.json()) as { token?: string };
    if (body.token) {
      window.localStorage.setItem(storageKey("token"), body.token);
      configureClient({ appName: "acme" });
    }
  } catch {
    // Network error — the submit handler will surface the failure.
  }
}

type Status = "idle" | "busy" | "success" | "error";

interface EarlyAccessFormProps {
  buttonLabel?: string;
  source?: string;
  compact?: boolean;
}

/**
 * EarlyAccessForm — inline email capture used in the hero and CTA sections.
 * Calls the `submitLead` function (auth:"guest") on the Pylon backend.
 */
export function EarlyAccessForm({
  buttonLabel = "Get early access",
  source = "hero",
  compact = false,
}: EarlyAccessFormProps) {
  const [email, setEmail] = useState("");
  const [status, setStatus] = useState<Status>("idle");
  const [error, setError] = useState<string | null>(null);
  const bootstrapped = useRef(false);

  useEffect(() => {
    if (bootstrapped.current) return;
    bootstrapped.current = true;
    void ensureGuest();
  }, []);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!email.trim() || status === "busy") return;
    setStatus("busy");
    setError(null);
    try {
      await ensureGuest();
      await callFn("submitLead", { email: email.trim(), source });
      setStatus("success");
      setEmail("");
    } catch (err) {
      setStatus("error");
      setError(
        err instanceof Error ? err.message : "Something went wrong. Try again.",
      );
    }
  }

  if (status === "success") {
    return (
      <p className="text-sm font-medium text-[var(--color-brand-deep)]">
        You're on the list. We'll be in touch.
      </p>
    );
  }

  return (
    <form
      onSubmit={(e) => void handleSubmit(e)}
      className={cn(
        "flex gap-2",
        compact ? "flex-col sm:flex-row" : "flex-col sm:flex-row",
      )}
    >
      <Input
        type="email"
        required
        value={email}
        onChange={(e) => setEmail(e.target.value)}
        placeholder="you@company.com"
        disabled={status === "busy"}
        className="h-10 min-w-0 flex-1 rounded-lg"
        aria-label="Work email"
      />
      <Button
        type="submit"
        disabled={status === "busy" || !email.trim()}
        className="h-10 whitespace-nowrap rounded-lg px-4 text-sm"
      >
        {status === "busy" ? "Sending…" : buttonLabel}
      </Button>
      {error && (
        <p className="col-span-full text-xs text-destructive">{error}</p>
      )}
    </form>
  );
}
