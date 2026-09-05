"use client";

import React, { useEffect, useState } from "react";
import { Link } from "@pylonsync/react";
import {
  passwordLogin,
  passwordRegister,
  persistSession,
  ApiError,
} from "@pylonsync/client";

// The email/password form shared by /login and /signup, plus an email-code
// path ("Email me a code") for people who never set a password. Every path
// calls the built-in auth API (`/api/auth/password/*`, `/api/auth/magic/*`).
//
// On success the server sets the session cookie AND returns the token; we
// call `persistSession` so the sync engine adopts the identity at once (a
// stale guest token in localStorage would otherwise shadow the cookie on the
// engine's calls). Sign-up then lands on /onboarding, which creates the
// workspace; sign-in lands on /dashboard.
export function AuthForm({ mode }: { mode: "login" | "signup" }) {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  const [codeSent, setCodeSent] = useState(false);
  const [method, setMethod] = useState<"password" | "code">("password");
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  // OAuth buttons render only for providers the server has configured
  // (GET /api/auth/providers). `null` while loading so nothing flashes in.
  const [providers, setProviders] = useState<string[] | null>(null);

  useEffect(() => {
    let alive = true;
    fetch("/api/auth/providers")
      .then((r) => (r.ok ? r.json() : []))
      .then((list: Array<{ provider: string }>) => {
        if (alive) setProviders(list.map((p) => p.provider));
      })
      .catch(() => {
        if (alive) setProviders([]);
      });
    return () => {
      alive = false;
    };
  }, []);

  const destination = mode === "login" ? "/dashboard" : "/onboarding";

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setPending(true);
    try {
      if (method === "code") {
        if (!codeSent) {
          await postJson("/api/auth/magic/send", { email });
          setCodeSent(true);
          setPending(false);
          return;
        }
        const session = await postJson<{ token: string; user_id?: string | null }>(
          "/api/auth/magic/verify",
          { email, code },
        );
        persistSession({ token: session.token, user_id: session.user_id ?? "" });
        // A code sign-in may have just created the account; the wizard
        // sends an already-onboarded workspace straight to the dashboard.
        window.location.assign("/onboarding");
        return;
      }
      const session =
        mode === "login"
          ? await passwordLogin({ email, password })
          : await passwordRegister({ email, password });
      persistSession(session);
      window.location.assign(destination);
    } catch (err) {
      setError(messageFor(err));
      setPending(false);
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
          disabled={codeSent}
        />
        {method === "password" ? (
          <IconField
            label="Password"
            type="password"
            icon={<LockIcon />}
            value={password}
            onChange={setPassword}
            required
            autoComplete={mode === "login" ? "current-password" : "new-password"}
            placeholder={mode === "login" ? "Enter your password" : "Choose a password (10+ characters)"}
            trailing={
              mode === "login" ? (
                <Link
                  href="/forgot-password"
                  className="text-[12px] text-zinc-500 underline underline-offset-2 hover:text-zinc-900"
                >
                  Forgot password?
                </Link>
              ) : null
            }
          />
        ) : codeSent ? (
          <IconField
            label="6-digit code"
            type="text"
            icon={<LockIcon />}
            value={code}
            onChange={(v) => setCode(v.replace(/\D/g, "").slice(0, 6))}
            required
            autoComplete="one-time-code"
            placeholder="123456"
            hint={`We emailed a code to ${email}.`}
          />
        ) : null}
        {mode === "signup" && method === "password" ? (
          <p className="text-[12px] leading-snug text-zinc-500">
            By joining, you agree to our{" "}
            <a href="/company/terms" className="underline underline-offset-2">
              Terms
            </a>{" "}
            &amp;{" "}
            <a href="/company/privacy" className="underline underline-offset-2">
              Privacy
            </a>
            .
          </p>
        ) : null}
        {error ? (
          <p className="rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-[13px] leading-snug text-red-700">
            {error}
          </p>
        ) : null}
        <button
          type="submit"
          disabled={pending || (method === "code" && codeSent && code.length !== 6)}
          className="inline-flex h-10 w-full items-center justify-center rounded-lg bg-zinc-900 text-sm font-medium text-white transition-colors hover:bg-zinc-700 disabled:opacity-60"
        >
          {pending
            ? "…"
            : method === "code"
              ? codeSent
                ? "Continue"
                : "Email me a code"
              : mode === "login"
                ? "Sign in"
                : "Create account"}
        </button>
      </form>

      <button
        type="button"
        onClick={() => {
          setMethod(method === "password" ? "code" : "password");
          setCodeSent(false);
          setCode("");
          setError(null);
        }}
        className="w-full text-center text-[13px] text-zinc-500 underline underline-offset-2 hover:text-zinc-900"
      >
        {method === "password" ? "Email me a sign-in code instead" : "Use a password instead"}
      </button>

      {/* Social sign-in — only for providers the server reports as
          configured. Enable Google with PYLON_OAUTH_GOOGLE_CLIENT_ID /
          _CLIENT_SECRET / _REDIRECT (GitHub: PYLON_OAUTH_GITHUB_*) and the
          button appears with zero code changes. */}
      {providers && providers.length > 0 ? (
        <>
          <div className="flex items-center gap-3 text-[11px] uppercase tracking-wide text-zinc-400">
            <span className="h-px flex-1 bg-zinc-200" />
            or continue with
            <span className="h-px flex-1 bg-zinc-200" />
          </div>

          {providers.includes("google") ? (
            <a
              href={`/api/auth/login/google?callback=${destination}&redirect=1`}
              className="inline-flex h-10 w-full items-center justify-center gap-2 rounded-lg border border-zinc-300 bg-white text-sm font-medium text-zinc-900 transition-colors hover:bg-zinc-50"
            >
              <GoogleIcon />
              Google
            </a>
          ) : null}
          {providers.includes("github") ? (
            <a
              href={`/api/auth/login/github?callback=${destination}&redirect=1`}
              className="inline-flex h-10 w-full items-center justify-center gap-2 rounded-lg border border-zinc-300 bg-white text-sm font-medium text-zinc-900 transition-colors hover:bg-zinc-50"
            >
              <GitHubIcon />
              GitHub
            </a>
          ) : null}
        </>
      ) : null}
    </div>
  );
}

async function postJson<T = Record<string, unknown>>(path: string, body: unknown): Promise<T> {
  const res = await fetch(path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    credentials: "include",
    body: JSON.stringify(body),
  });
  const data = (await res.json().catch(() => ({}))) as T & {
    error?: { code?: string; message?: string };
  };
  if (!res.ok) {
    throw new Error(data.error?.message ?? `Request failed (${res.status})`);
  }
  return data;
}

function IconField({
  label,
  icon,
  value,
  onChange,
  trailing,
  hint,
  ...rest
}: {
  label: string;
  icon: React.ReactNode;
  value: string;
  onChange: (v: string) => void;
  trailing?: React.ReactNode;
  hint?: string;
} & Omit<React.InputHTMLAttributes<HTMLInputElement>, "value" | "onChange">) {
  return (
    <label className="block">
      <span className="mb-1.5 flex items-center justify-between text-[13px] font-medium text-zinc-700">
        {label}
        {trailing}
      </span>
      <span className="relative block">
        <span className="pointer-events-none absolute inset-y-0 left-3 flex items-center text-zinc-400">
          {icon}
        </span>
        <input
          value={value}
          onChange={(e) => onChange(e.target.value)}
          className="h-10 w-full rounded-lg border border-zinc-300 bg-white pl-9 pr-3 text-sm text-zinc-900 outline-none transition placeholder:text-zinc-400 focus:border-zinc-900 focus:ring-2 focus:ring-zinc-900/10 disabled:bg-zinc-50 disabled:text-zinc-500"
          {...rest}
        />
      </span>
      {hint ? <span className="mt-1.5 block text-[12px] text-zinc-500">{hint}</span> : null}
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
      <rect x="4" y="11" width="16" height="10" rx="2" />
      <path d="M8 11V7a4 4 0 0 1 8 0v4" />
    </svg>
  );
}

function GitHubIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
      <path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12" />
    </svg>
  );
}

function GoogleIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden>
      <path fill="#4285F4" d="M23.49 12.27c0-.79-.07-1.54-.19-2.27H12v4.51h6.47a5.53 5.53 0 0 1-2.4 3.63v3.01h3.87c2.27-2.09 3.55-5.17 3.55-8.88z" />
      <path fill="#34A853" d="M12 24c3.24 0 5.95-1.08 7.94-2.91l-3.87-3.01c-1.08.72-2.45 1.16-4.07 1.16-3.13 0-5.78-2.11-6.73-4.96H1.29v3.09A12 12 0 0 0 12 24z" />
      <path fill="#FBBC05" d="M5.27 14.28A7.2 7.2 0 0 1 4.89 12c0-.79.14-1.56.38-2.28V6.63H1.29A12 12 0 0 0 0 12c0 1.94.46 3.77 1.29 5.37l3.98-3.09z" />
      <path fill="#EA4335" d="M12 4.75c1.77 0 3.35.61 4.6 1.8l3.44-3.44C17.95 1.19 15.24 0 12 0A12 12 0 0 0 1.29 6.63l3.98 3.09C6.22 6.86 8.87 4.75 12 4.75z" />
    </svg>
  );
}

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
