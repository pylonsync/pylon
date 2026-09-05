"use client";

import React, { useState } from "react";
import { persistSession } from "@pylonsync/client";
import { Link } from "@pylonsync/react";

const inputCls =
  "h-10 w-full rounded-lg border border-zinc-300 bg-white px-3 text-sm text-zinc-900 outline-none transition placeholder:text-zinc-400 focus:border-zinc-900 focus:ring-2 focus:ring-zinc-900/10";
const buttonCls =
  "inline-flex h-10 w-full items-center justify-center rounded-lg bg-zinc-900 text-sm font-medium text-white transition-colors hover:bg-zinc-700 disabled:opacity-60";

async function post(path: string, body: unknown): Promise<Record<string, unknown>> {
  const res = await fetch(path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    credentials: "include",
    body: JSON.stringify(body),
  });
  const data = (await res.json().catch(() => ({}))) as {
    error?: { code?: string; message?: string };
  };
  if (!res.ok) throw new Error(data.error?.message ?? `Request failed (${res.status})`);
  return data as Record<string, unknown>;
}

/** /forgot-password: request a reset email. Always says "sent" so the form
 *  does not reveal whether an address has an account. */
export function ForgotPasswordForm() {
  const [email, setEmail] = useState("");
  const [sent, setSent] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setPending(true);
    setError(null);
    try {
      await post("/api/auth/password/reset/request", { email });
      setSent(true);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Something went wrong.");
    } finally {
      setPending(false);
    }
  }

  if (sent) {
    return (
      <p className="text-sm text-zinc-600">
        If an account exists for <span className="font-medium">{email}</span>, a reset link is on its way. The
        link expires in one hour.
      </p>
    );
  }
  return (
    <form onSubmit={onSubmit} className="space-y-4">
      <input
        type="email"
        required
        autoComplete="email"
        value={email}
        onChange={(e) => setEmail(e.target.value)}
        placeholder="you@company.com"
        aria-label="Email"
        className={inputCls}
      />
      {error && <p className="text-[13px] text-red-700">{error}</p>}
      <button type="submit" disabled={pending} className={buttonCls}>
        {pending ? "…" : "Send reset link"}
      </button>
      <p className="text-center text-[13px] text-zinc-500">
        <Link href="/login" className="underline underline-offset-2">
          Back to sign in
        </Link>
      </p>
    </form>
  );
}

/** /reset-password?token=…: set a new password and sign in. */
export function ResetPasswordForm({ token }: { token: string }) {
  const [password, setPassword] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setPending(true);
    setError(null);
    try {
      const data = await post("/api/auth/password/reset/complete", { token, newPassword: password });
      if (typeof data.token === "string") {
        persistSession({ token: data.token, user_id: (data.user_id as string) ?? null });
        window.location.assign("/dashboard");
        return;
      }
      window.location.assign("/login?reset=1");
    } catch (err) {
      setError(err instanceof Error ? err.message : "That link is no longer valid.");
      setPending(false);
    }
  }

  if (!token) {
    return (
      <p className="text-sm text-zinc-600">
        This reset link is missing its token.{" "}
        <Link href="/forgot-password" className="underline underline-offset-2">
          Request a new one
        </Link>
        .
      </p>
    );
  }
  return (
    <form onSubmit={onSubmit} className="space-y-4">
      <input
        type="password"
        required
        minLength={10}
        autoComplete="new-password"
        value={password}
        onChange={(e) => setPassword(e.target.value)}
        placeholder="New password (10+ characters)"
        aria-label="New password"
        className={inputCls}
      />
      {error && <p className="text-[13px] text-red-700">{error}</p>}
      <button type="submit" disabled={pending} className={buttonCls}>
        {pending ? "…" : "Set new password"}
      </button>
    </form>
  );
}
