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
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setPending(true);
    try {
      if (mode === "login") {
        await passwordLogin({ email, password });
        // Full navigation: the SSR dashboard re-renders with the new cookie.
        window.location.assign("/dashboard");
      } else {
        await passwordRegister({ email, password });
        // New accounts have no workspace yet — send them through first-run
        // onboarding (which redirects to /dashboard once they're in an org).
        window.location.assign("/onboarding");
      }
    } catch (err) {
      setError(messageFor(err));
      setPending(false); // keep the form up to retry (success navigates away)
    }
  }

  return (
    <div className="space-y-5">
      <form onSubmit={onSubmit} className="space-y-4">
        <IconField
          label="Email"
          type="email"
          icon={<MailIcon />}
          value={email}
          onChange={setEmail}
          required
          autoComplete="email"
          placeholder="Enter your email"
        />
        <IconField
          label="Password"
          type="password"
          icon={<LockIcon />}
          value={password}
          onChange={setPassword}
          required
          autoComplete={mode === "login" ? "current-password" : "new-password"}
          placeholder="Enter your password"
        />
        {mode === "signup" ? (
          <p className="text-[12px] leading-snug text-zinc-500">
            By joining, you agree to our{" "}
            <a href="/company/privacy" className="underline underline-offset-2">
              Terms
            </a>{" "}
            &amp;{" "}
            <a href="/company/privacy" className="underline underline-offset-2">
              Privacy
            </a>
            . Passwords need 10+ characters.
          </p>
        ) : null}
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
          {pending ? "…" : mode === "login" ? "Sign in" : "Sign up"}
        </button>
      </form>

      <div className="flex items-center gap-3 text-[11px] uppercase tracking-wide text-zinc-400">
        <span className="h-px flex-1 bg-zinc-200" />
        or continue with
        <span className="h-px flex-1 bg-zinc-200" />
      </div>

      {/* Social sign-in. The Google provider must be configured (set
          PYLON_OAUTH_GOOGLE_CLIENT_ID / _CLIENT_SECRET / _REDIRECT) — until then
          this button returns a helpful "configure the provider" error. */}
      <a
        href="/api/auth/login/google?callback=/dashboard&redirect=1"
        className="inline-flex h-10 w-full items-center justify-center gap-2 rounded-lg border border-zinc-300 bg-white text-sm font-medium text-zinc-900 transition-colors hover:bg-zinc-50"
      >
        <GoogleIcon />
        Google
      </a>
    </div>
  );
}

function IconField({
  label,
  icon,
  value,
  onChange,
  type = "text",
  required,
  autoComplete,
  placeholder,
}: {
  label: string;
  icon: React.ReactNode;
  value: string;
  onChange: (v: string) => void;
  type?: string;
  required?: boolean;
  autoComplete?: string;
  placeholder?: string;
}) {
  return (
    <label className="block">
      <span className="mb-1.5 block text-[13px] font-medium text-zinc-700">
        {label}
      </span>
      <div className="relative">
        <span className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-zinc-400">
          {icon}
        </span>
        <input
          type={type}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          required={required}
          autoComplete={autoComplete}
          placeholder={placeholder}
          className="h-10 w-full rounded-lg border border-zinc-300 bg-white pl-9 pr-3 text-sm text-zinc-900 outline-none transition placeholder:text-zinc-400 focus:border-zinc-900 focus:ring-2 focus:ring-zinc-900/10"
        />
      </div>
    </label>
  );
}

function MailIcon() {
  return (
    <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
      <rect x="3" y="5" width="18" height="14" rx="2" />
      <path d="m3 7 9 6 9-6" />
    </svg>
  );
}

function LockIcon() {
  return (
    <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
      <rect x="5" y="11" width="14" height="10" rx="2" />
      <path d="M8 11V7a4 4 0 0 1 8 0v4" />
    </svg>
  );
}

function GoogleIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden>
      <path
        fill="#4285F4"
        d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92c-.26 1.37-1.04 2.53-2.21 3.31v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.09z"
      />
      <path
        fill="#34A853"
        d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z"
      />
      <path
        fill="#FBBC05"
        d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l3.66-2.84z"
      />
      <path
        fill="#EA4335"
        d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z"
      />
    </svg>
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
