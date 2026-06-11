"use client";

import React, { useEffect, useRef, useState } from "react";
import { callFn, configureClient, init, storageKey } from "@pylonsync/react";
import { Button } from "@pylonsync/example-ui/button";
import { Input } from "@pylonsync/example-ui/input";
import { Label } from "@pylonsync/example-ui/label";
import { Textarea } from "@pylonsync/example-ui/textarea";

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

export function ContactPageForm() {
  const [email, setEmail] = useState("");
  const [message, setMessage] = useState("");
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
      await callFn("submitLead", {
        email: email.trim(),
        message: message.trim(),
        source: "contact",
      });
      setStatus("success");
      setEmail("");
      setMessage("");
    } catch (err) {
      setStatus("error");
      setError(
        err instanceof Error ? err.message : "Something went wrong. Please try again.",
      );
    }
  }

  if (status === "success") {
    return (
      <div className="rounded-2xl border bg-card p-8 text-center">
        <p className="text-2xl">✓</p>
        <p className="mt-3 text-sm font-medium text-foreground">
          Got it. We'll be in touch within one business day.
        </p>
      </div>
    );
  }

  return (
    <form
      onSubmit={(e) => void handleSubmit(e)}
      className="flex flex-col gap-5 rounded-2xl border bg-card p-8"
    >
      <div className="grid gap-1.5">
        <Label htmlFor="contact-email" className="text-sm font-medium">
          Work email
        </Label>
        <Input
          id="contact-email"
          type="email"
          required
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          placeholder="you@company.com"
          disabled={status === "busy"}
        />
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor="contact-message" className="text-sm font-medium">
          Message{" "}
          <span className="text-muted-foreground font-normal">(optional)</span>
        </Label>
        <Textarea
          id="contact-message"
          value={message}
          onChange={(e) => setMessage(e.target.value)}
          placeholder="What can we help you with?"
          rows={4}
          disabled={status === "busy"}
        />
      </div>
      {error && (
        <p className="text-xs text-destructive">{error}</p>
      )}
      <Button
        type="submit"
        disabled={status === "busy" || !email.trim()}
        className="w-full"
      >
        {status === "busy" ? "Sending…" : "Send message"}
      </Button>
    </form>
  );
}
