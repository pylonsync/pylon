"use client";

import React, { useState } from "react";
import { cn } from "@/lib/utils";
import { Spinner } from "@/components/ui/spinner";
import { useAuth } from "./MarketProvider";
import { DEMO } from "./market";

const INPUT =
  "h-11 w-full rounded-xl bg-background px-3.5 text-[15px] ring-1 ring-input outline-none placeholder:text-muted-foreground focus:ring-2 focus:ring-ring focus-visible:outline-none";

// The sign-in form shown wherever a write needs a real user (sell, make an
// offer, the dashboard). Email and password, no verification email.
// Prefilled with the seeded demo account.
export function LoginCard({
  title = "Sign in to continue",
  blurb = "Use the demo account or create your own. No verification email.",
  headingLevel = 2,
  onDone,
  className,
}: {
  title?: string;
  blurb?: string;
  headingLevel?: 1 | 2;
  /** Called after a successful sign-in or sign-up. */
  onDone?: () => void;
  className?: string;
}) {
  const { signIn, signUp } = useAuth();
  const [mode, setMode] = useState<"login" | "register">("login");
  const [email, setEmail] = useState<string>(DEMO.email);
  const [password, setPassword] = useState<string>(DEMO.password);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const Heading = headingLevel === 1 ? "h1" : "h2";

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setErr(null);
    try {
      if (mode === "login") await signIn(email, password);
      else await signUp(email, password, name);
      onDone?.();
    } catch (e) {
      setErr((e as Error).message ?? "Sign-in failed");
    } finally {
      setBusy(false);
    }
  }

  function switchMode(next: "login" | "register") {
    setMode(next);
    setErr(null);
    setEmail(next === "login" ? DEMO.email : "");
    setPassword(next === "login" ? DEMO.password : "");
  }

  return (
    <div
      className={cn(
        "market-rise mx-auto w-full max-w-sm rounded-2xl bg-card p-6 shadow-[var(--shadow-border)]",
        className,
      )}
    >
      <Heading className="font-display text-[28px] leading-tight">{title}</Heading>
      <p className="mt-1.5 text-sm leading-5 text-muted-foreground">{blurb}</p>
      <form onSubmit={submit} className="mt-5 flex flex-col gap-3.5">
        {mode === "register" ? (
          <div className="flex flex-col gap-1.5">
            <label htmlFor="lc-name" className="text-sm font-medium">
              Name
            </label>
            <input
              id="lc-name"
              name="name"
              autoComplete="name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Pat Lee"
              className={INPUT}
            />
          </div>
        ) : null}
        <div className="flex flex-col gap-1.5">
          <label htmlFor="lc-email" className="text-sm font-medium">
            Email
          </label>
          <input
            id="lc-email"
            name="email"
            type="email"
            required
            autoComplete="email"
            spellCheck={false}
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            placeholder="you@example.com"
            className={INPUT}
          />
        </div>
        <div className="flex flex-col gap-1.5">
          <label htmlFor="lc-password" className="text-sm font-medium">
            Password
          </label>
          <input
            id="lc-password"
            name="password"
            type="password"
            required
            minLength={8}
            autoComplete={mode === "login" ? "current-password" : "new-password"}
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder={mode === "register" ? "8 or more characters" : ""}
            className={INPUT}
          />
        </div>
        <div aria-live="polite">
          {err ? (
            <p className="rounded-xl bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {err}
            </p>
          ) : null}
        </div>
        <button
          type="submit"
          disabled={busy}
          className="inline-flex min-h-11 items-center justify-center gap-2 rounded-full bg-primary px-5 text-sm font-medium text-primary-foreground transition-[scale] active:scale-[0.98] disabled:opacity-60"
        >
          {busy ? <Spinner /> : null}
          {mode === "login"
            ? busy
              ? "Logging in"
              : "Log in"
            : busy
              ? "Creating account"
              : "Create account"}
        </button>
      </form>
      <p className="mt-4 text-center text-xs text-muted-foreground">
        {mode === "login" ? "No account?" : "Have an account?"}{" "}
        <button
          type="button"
          onClick={() => switchMode(mode === "login" ? "register" : "login")}
          className="font-medium text-foreground underline-offset-4 hover:underline"
        >
          {mode === "login" ? "Sign up" : "Log in"}
        </button>
      </p>
    </div>
  );
}
