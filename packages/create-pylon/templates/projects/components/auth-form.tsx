"use client";

import React, { useState } from "react";
import {
  ApiError,
  passwordLogin,
  passwordRegister,
  persistSession,
} from "@pylonsync/client";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

/**
 * One form, two modes. Calls the built-in auth API (`/api/auth/password/*`),
 * then `persistSession` writes the token so the sync engine and `callFn`
 * authenticate as this user on the next load.
 *
 * No navigation afterwards: `persistSession` notifies the sync engine and the
 * client-side gate swaps the app in. Reloading would hand the decision to the
 * SSR cookie, which browsers withhold inside the builder's cross-site preview
 * iframe — that's the sign-in loop this avoids.
 */
export function AuthForm() {
  const [mode, setMode] = useState<"login" | "signup">("login");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  async function onSubmit(event: React.FormEvent) {
    event.preventDefault();
    setError(null);
    setPending(true);
    try {
      const session =
        mode === "login"
          ? await passwordLogin({ email, password })
          : await passwordRegister({ email, password });
      // No navigation: persistSession notifies the sync engine, RequireAuth
      // re-renders, and the app appears in place. A full page load here would
      // depend on the SSR session cookie, which a cross-site iframe withholds.
      persistSession(session);
    } catch (err) {
      setError(messageFor(err));
      setPending(false);
    }
  }

  return (
    <div className="space-y-5">
      <form onSubmit={onSubmit} className="space-y-3.5">
        <div className="space-y-1.5">
          <Label htmlFor="email">Email</Label>
          <Input
            id="email"
            type="email"
            value={email}
            onChange={(event) => setEmail(event.target.value)}
            required
            autoComplete="email"
            placeholder="you@company.com"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="password">Password</Label>
          <Input
            id="password"
            type="password"
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            required
            autoComplete={mode === "login" ? "current-password" : "new-password"}
            placeholder={mode === "login" ? "Your password" : "At least 10 characters"}
          />
        </div>
        {error ? (
          <p className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-[12px] leading-snug text-destructive">
            {error}
          </p>
        ) : null}
        <Button type="submit" disabled={pending} className="w-full">
          {pending ? "…" : mode === "login" ? "Sign in" : "Create account"}
        </Button>
      </form>

      <p className="text-center text-[12px] text-muted-foreground">
        {mode === "login" ? "First time here?" : "Already have an account?"}{" "}
        <button
          type="button"
          onClick={() => {
            setMode(mode === "login" ? "signup" : "login");
            setError(null);
          }}
          className="font-medium text-foreground underline underline-offset-2"
        >
          {mode === "login" ? "Create an account" : "Sign in"}
        </button>
      </p>
    </div>
  );
}

// `ApiError` carries a stable `.code`, so branch on the code rather than the
// message — the copy here is ours, the message is the framework's.
function messageFor(err: unknown): string {
  if (err instanceof ApiError) {
    switch (err.code) {
      case "INVALID_CREDENTIALS":
        return "Wrong email or password.";
      case "USER_EXISTS":
        return "That email is already registered — sign in instead.";
      case "WEAK_PASSWORD":
        return "Pick a longer password — at least 10 characters.";
      case "PWNED_PASSWORD":
        return "That password has appeared in a known breach. Choose another.";
      case "RATE_LIMITED":
        return "Too many attempts — try again in a minute.";
      default:
        return err.message;
    }
  }
  if (err instanceof Error) return err.message;
  return "Something went wrong. Try again.";
}
