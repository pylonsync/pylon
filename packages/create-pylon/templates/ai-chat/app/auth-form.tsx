"use client";

import React, { useState } from "react";
import { callFn } from "@pylonsync/react";
import { ApiError, passwordLogin, passwordRegister, persistSession } from "@pylonsync/client";

// Email and password sign-in, in two modes. Before signing in, a guest asks
// the server for a one-time claim code (`prepareGuestClaim`). After the new
// session is stored, `claimGuestHistory` uses the code to move the guest's
// conversations to the account. Then a full page load lets the server render
// with the new session cookie.
export function AuthForm({ next }: { next: string }) {
  const [mode, setMode] = useState<"login" | "register">("login");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setPending(true);

    let claimCode: string | null = null;
    try {
      const r = await callFn<{ code: string | null }>("prepareGuestClaim");
      claimCode = r.code;
    } catch {
      /* no guest session: nothing to move */
    }

    try {
      const session =
        mode === "login" ? await passwordLogin({ email, password }) : await passwordRegister({ email, password });
      persistSession(session);
      if (claimCode) {
        try {
          await callFn("claimGuestHistory", { code: claimCode }, { token: session.token });
        } catch {
          /* sign-in succeeded; the guest chats stay with the guest session */
        }
      }
      window.location.assign(next);
    } catch (err) {
      setError(messageFor(err));
      setPending(false);
    }
  }

  const inputClass =
    "h-11 w-full rounded-xl bg-bg px-3.5 text-[15px] text-ink outline-none ring-1 ring-line transition-shadow duration-200 placeholder:text-ink-3 focus:ring-2 focus:ring-brand/40";

  return (
    <div className="space-y-5">
      <form onSubmit={onSubmit} className="space-y-4">
        <label className="block">
          <span className="mb-1.5 block text-[13px] font-medium text-ink-2">Email</span>
          <input
            type="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            required
            autoComplete="email"
            placeholder="you@example.com"
            className={inputClass}
          />
        </label>
        <label className="block">
          <span className="mb-1.5 block text-[13px] font-medium text-ink-2">Password</span>
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
            autoComplete={mode === "login" ? "current-password" : "new-password"}
            placeholder={mode === "login" ? "Your password" : "At least 10 characters"}
            className={inputClass}
          />
        </label>
        {error ? (
          <p className="rounded-xl bg-danger/8 px-3.5 py-2.5 text-[13px] leading-snug text-danger ring-1 ring-danger/15" role="alert">
            {error}
          </p>
        ) : null}
        <button
          type="submit"
          disabled={pending}
          className="press inline-flex h-11 w-full items-center justify-center rounded-full bg-ink text-[14.5px] font-medium text-bg transition-opacity duration-200 hover:opacity-90 disabled:opacity-60"
        >
          {pending ? "Signing in" : mode === "login" ? "Sign in" : "Create account"}
        </button>
      </form>

      <p className="text-center text-[13.5px] text-ink-2">
        {mode === "login" ? "New here?" : "Already have an account?"}{" "}
        <button
          type="button"
          onClick={() => {
            setMode(mode === "login" ? "register" : "login");
            setError(null);
          }}
          className="font-medium text-ink underline decoration-ink/25 underline-offset-4 hover:decoration-ink"
        >
          {mode === "login" ? "Create an account" : "Sign in"}
        </button>
      </p>
    </div>
  );
}

// Maps the framework's auth error codes to messages.
function messageFor(err: unknown): string {
  if (err instanceof ApiError) {
    switch (err.code) {
      case "INVALID_CREDENTIALS":
        return "Wrong email or password.";
      case "USER_EXISTS":
        return "That email is already registered. Sign in instead.";
      case "WEAK_PASSWORD":
        return "Use at least 10 characters.";
      case "PWNED_PASSWORD":
        return "That password appears in a known data breach. Choose a different one.";
      case "RATE_LIMITED":
        return "Too many attempts. Try again in a minute.";
      default:
        return err.message;
    }
  }
  if (err instanceof Error) return err.message;
  return "Something went wrong. Try again.";
}
