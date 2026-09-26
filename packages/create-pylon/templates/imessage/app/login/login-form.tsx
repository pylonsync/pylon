"use client";

import React, { useState } from "react";
import { ApiError, persistSession } from "@pylonsync/client";

async function post<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    credentials: "include",
    body: JSON.stringify(body),
  });
  const json = (await res.json().catch(() => ({}))) as T & { error?: { code?: string; message?: string } };
  if (!res.ok || json.error) {
    throw new ApiError(json.error?.code ?? `HTTP_${res.status}`, json.error?.message ?? "Request failed", res.status);
  }
  return json;
}

// Email code sign-in (POST /api/auth/magic/send, then /verify). The verified
// code is what lets Pylon promote the owner email to admin.
export function LoginForm() {
  const [email, setEmail] = useState("");
  const [code, setCode] = useState("");
  const [sent, setSent] = useState(false);
  const [devCode, setDevCode] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setPending(true);
    try {
      if (!sent) {
        const res = await post<{ dev_code?: string }>("/api/auth/magic/send", { email });
        // `pylon dev` returns the code in the response instead of emailing it.
        setDevCode(res.dev_code ?? null);
        setSent(true);
        setPending(false);
        return;
      }
      const session = await post<{ token: string; user_id?: string | null }>("/api/auth/magic/verify", {
        email,
        code: code.trim(),
      });
      persistSession({ token: session.token, user_id: session.user_id ?? "" });
      // An identity change: load a fresh document so the server renders as the owner.
      window.location.assign("/");
    } catch (err) {
      setError(err instanceof ApiError && err.code === "INVALID_CODE" ? "That code is wrong or expired." : err instanceof Error ? err.message : "Sign-in failed");
      setPending(false);
    }
  }

  const input =
    "h-10 w-full rounded-xl border bg-background px-3.5 text-[15px] outline-none transition-shadow focus:shadow-[0_0_0_4px_rgba(10,132,255,0.14)] focus:border-bubble-out/50";

  return (
    <form onSubmit={onSubmit} className="space-y-3">
      <label className="block">
        <span className="mb-1.5 block text-[13px] font-medium">Email</span>
        <input
          type="email"
          required
          autoComplete="email"
          value={email}
          disabled={sent}
          onChange={(e) => setEmail(e.target.value)}
          placeholder="you@example.com"
          className={input}
        />
      </label>
      {sent ? (
        <label className="block">
          <span className="mb-1.5 block text-[13px] font-medium">Code</span>
          <input
            inputMode="numeric"
            autoComplete="one-time-code"
            required
            autoFocus
            value={code}
            onChange={(e) => setCode(e.target.value)}
            placeholder="123456"
            className={`${input} font-mono tracking-[0.3em]`}
          />
          <span className="mt-1.5 block text-[12px] text-muted-foreground">
            {devCode ? `Development server: your code is ${devCode}.` : `Sent to ${email}. It expires in 10 minutes.`}
          </span>
        </label>
      ) : null}
      {error ? <p className="rounded-xl bg-destructive/10 px-3 py-2 text-[13px] text-destructive">{error}</p> : null}
      <button
        type="submit"
        disabled={pending}
        className="h-10 w-full rounded-full bg-bubble-out text-[14px] font-medium text-white transition-transform duration-300 ease-[cubic-bezier(0.32,0.72,0,1)] active:scale-[0.98] disabled:opacity-60"
      >
        {pending ? "One moment" : sent ? "Sign in" : "Send code"}
      </button>
      {sent ? (
        <button
          type="button"
          onClick={() => {
            setSent(false);
            setCode("");
            setDevCode(null);
          }}
          className="w-full text-[13px] text-muted-foreground hover:text-foreground"
        >
          Use a different email
        </button>
      ) : null}
    </form>
  );
}
