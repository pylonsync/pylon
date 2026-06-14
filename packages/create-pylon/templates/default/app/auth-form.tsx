"use client";

import React, { useState } from "react";
import { passwordLogin, passwordRegister, ApiError } from "@pylonsync/client";
import { Button } from "@/components/ui/button";

// The email/password form, shared by /login and /signup. It calls the built-in
// auth API directly — `passwordLogin` / `passwordRegister` (from
// @pylonsync/client) POST to `/api/auth/password/*`.
//
// On success the server sets an HttpOnly session cookie on the response. We do
// a full navigation to /dashboard rather than a client transition: the fresh
// page load hands that cookie to the SSR runtime (which resolves auth and
// renders the dashboard server-side) and to the sync engine (which
// authenticates with the same cookie via `credentials: include`). Because the
// cookie is HttpOnly it can never be read by JavaScript, so there is no
// session token sitting in `localStorage` for an XSS to lift. (Cross-origin or
// native clients, which can't rely on the cookie, use the token-based path via
// `persistSession` instead — not needed here, same origin.)
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
        placeholder={mode === "signup" ? "at least 8 characters" : undefined}
      />
      {error ? (
        <p className="rounded-md border border-red-600/30 bg-red-600/10 px-3 py-2 text-sm text-red-700">
          {error}
        </p>
      ) : null}
      <Button type="submit" disabled={pending} className="w-full">
        {pending ? "…" : mode === "login" ? "Sign in" : "Create account"}
      </Button>
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
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  type?: string;
  required?: boolean;
  autoComplete?: string;
  placeholder?: string;
}) {
  return (
    <label className="block space-y-1.5">
      <span className="text-sm font-medium">{label}</span>
      <input
        type={type}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        required={required}
        autoComplete={autoComplete}
        placeholder={placeholder}
        className="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm outline-none focus-visible:ring-1 focus-visible:ring-ring"
      />
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
        return "Pick a stronger password (at least 8 characters).";
      case "RATE_LIMITED":
        return "Too many attempts — try again in a minute.";
      default:
        return err.message;
    }
  }
  if (err instanceof Error) return err.message;
  return "Something went wrong. Try again.";
}
