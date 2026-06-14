"use client";

import React, { useState } from "react";
import { passwordLogin, passwordRegister, ApiError } from "@pylonsync/client";

// The email/password form, shared by /login and /signup. It calls the built-in
// auth API directly — `passwordLogin` / `passwordRegister` (from
// @pylonsync/client) POST to `/api/auth/password/*`.
//
// On success the server sets an HttpOnly session cookie on the response. We do
// a full navigation to /dashboard rather than a client transition: the fresh
// page load hands that cookie to the SSR runtime (which resolves auth and
// renders the dashboard server-side) and to the sync engine (which
// authenticates with the same cookie via `credentials: include`). Because the
// cookie is HttpOnly it can never be read by JavaScript, so there is no session
// token sitting in `localStorage` for an XSS to lift.
export function AuthForm({ mode }: { mode: "login" | "signup" }) {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setPending(true);
    try {
      if (mode === "login") {
        await passwordLogin({ email, password });
      } else {
        await passwordRegister({
          email,
          password,
          displayName: displayName.trim() || undefined,
        });
      }
      // Full navigation: the SSR dashboard re-renders with the new cookie.
      window.location.assign("/dashboard");
    } catch (err) {
      setError(messageFor(err));
      setPending(false); // keep the form up to retry (success navigates away)
    }
  }

  return (
    <form onSubmit={onSubmit} className="space-y-4">
      {mode === "signup" ? (
        <Field
          label="Name"
          value={displayName}
          onChange={setDisplayName}
          autoComplete="name"
          placeholder="optional"
        />
      ) : null}
      <Field
        label="Email"
        type="email"
        value={email}
        onChange={setEmail}
        required
        autoComplete="email"
        placeholder="you@example.com"
      />
      <Field
        label="Password"
        type="password"
        value={password}
        onChange={setPassword}
        required
        autoComplete={mode === "login" ? "current-password" : "new-password"}
        placeholder={mode === "signup" ? "at least 10 characters" : undefined}
        hint={
          mode === "signup"
            ? "At least 10 characters. Common or breached passwords are rejected."
            : undefined
        }
      />
      {error ? (
        <p className="rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-[13px] leading-snug text-red-700">
          {error}
        </p>
      ) : null}
      <button
        type="submit"
        disabled={pending}
        className="inline-flex h-10 w-full items-center justify-center rounded-full bg-zinc-900 text-sm font-medium text-white transition-colors hover:bg-zinc-700 disabled:opacity-60"
      >
        {pending ? "…" : mode === "login" ? "Sign in" : "Create account"}
      </button>
    </form>
  );
}

function Field({
  label,
  value,
  onChange,
  type = "text",
  required,
  autoComplete,
  placeholder,
  hint,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  type?: string;
  required?: boolean;
  autoComplete?: string;
  placeholder?: string;
  hint?: string;
}) {
  return (
    <label className="block space-y-1.5">
      <span className="text-[13px] font-medium text-zinc-700">{label}</span>
      <input
        type={type}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        required={required}
        autoComplete={autoComplete}
        placeholder={placeholder}
        className="h-10 w-full rounded-lg border border-zinc-300 bg-white px-3 text-sm text-zinc-900 outline-none transition placeholder:text-zinc-400 focus:border-zinc-900 focus:ring-2 focus:ring-zinc-900/10"
      />
      {hint ? <span className="block text-[12px] text-zinc-400">{hint}</span> : null}
    </label>
  );
}

// Map the framework's auth error codes to friendly copy. `ApiError` carries a
// stable `.code` (and `.status`) so you branch on the code, not the message.
function messageFor(err: unknown): string {
  if (err instanceof ApiError) {
    switch (err.code) {
      case "INVALID_CREDENTIALS":
        return "Wrong email or password.";
      case "USER_EXISTS":
        return "That email is already in use — sign in instead.";
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
