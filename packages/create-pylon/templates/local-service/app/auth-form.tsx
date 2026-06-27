"use client";

import React, { useState } from "react";
import {
  passwordLogin,
  passwordRegister,
  persistSession,
  ApiError,
} from "@pylonsync/client";

// The owner's email/password form — one form, two modes. It calls the built-in
// auth API directly (`passwordLogin` / `passwordRegister` POST to
// `/api/auth/password/*`), then `persistSession` writes the freshly-minted
// token to local storage so the sync engine + `callFn` authenticate AS THE
// OWNER on the next load. This step matters here specifically: the landing page
// mints an anonymous guest session (for the live booking picker), and without
// persisting the real session that stale guest token would shadow the owner's
// — so the owner-only `bookingsForOwner` call would come back as a guest and get
// rejected. We then do a full navigation to /dashboard so the SSR runtime
// re-resolves auth from the HttpOnly cookie and renders server-side.
//
// A booking site is single-tenant: there's no public signup funnel, just the owner
// creating their one account. Whoever signs in only sees data if their email
// matches PYLON_OWNER_EMAIL — enforced by the bookingsForOwner function.
export function AuthForm() {
  const [mode, setMode] = useState<"login" | "signup">("login");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setPending(true);
    try {
      const session =
        mode === "login"
          ? await passwordLogin({ email, password })
          : await passwordRegister({ email, password });
      // Make this session authoritative, replacing any anonymous guest token.
      persistSession(session);
      window.location.assign("/dashboard");
    } catch (err) {
      setError(messageFor(err));
      setPending(false);
    }
  }

  return (
    <div className="space-y-5">
      <form onSubmit={onSubmit} className="space-y-4">
        <label className="block">
          <span className="mb-1.5 block text-[13px] font-medium text-zinc-700">Email</span>
          <input
            type="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            required
            autoComplete="email"
            placeholder="you@yourbusiness.com"
            className="h-10 w-full rounded-lg border border-zinc-300 bg-white px-3 text-sm text-zinc-900 outline-none transition placeholder:text-zinc-400 focus:border-brand focus:ring-2 focus:ring-brand/20"
          />
        </label>
        <label className="block">
          <span className="mb-1.5 block text-[13px] font-medium text-zinc-700">Password</span>
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
            autoComplete={mode === "login" ? "current-password" : "new-password"}
            placeholder={mode === "login" ? "Your password" : "At least 10 characters"}
            className="h-10 w-full rounded-lg border border-zinc-300 bg-white px-3 text-sm text-zinc-900 outline-none transition placeholder:text-zinc-400 focus:border-brand focus:ring-2 focus:ring-brand/20"
          />
        </label>
        {error ? (
          <p className="rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-[13px] leading-snug text-red-700">
            {error}
          </p>
        ) : null}
        <button
          type="submit"
          disabled={pending}
          className="inline-flex h-10 w-full items-center justify-center rounded-lg bg-zinc-900 text-sm font-medium text-white transition-colors hover:bg-zinc-700 disabled:opacity-60"
        >
          {pending ? "…" : mode === "login" ? "Sign in" : "Create account"}
        </button>
      </form>

      <p className="text-center text-[13px] text-zinc-500">
        {mode === "login" ? "First time here?" : "Already have an account?"}{" "}
        <button
          type="button"
          onClick={() => {
            setMode(mode === "login" ? "signup" : "login");
            setError(null);
          }}
          className="font-medium text-zinc-900 underline underline-offset-2"
        >
          {mode === "login" ? "Create the owner account" : "Sign in"}
        </button>
      </p>
    </div>
  );
}

// Map the framework's auth error codes to friendly copy. `ApiError` carries a
// stable `.code` so you branch on the code, not the message.
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
        return "That password has appeared in a known data breach. Choose a different one.";
      case "RATE_LIMITED":
        return "Too many attempts — try again in a minute.";
      default:
        return err.message;
    }
  }
  if (err instanceof Error) return err.message;
  return "Something went wrong. Try again.";
}
