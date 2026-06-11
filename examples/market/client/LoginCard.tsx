"use client";

import React, { useState } from "react";
import { Button } from "@pylonsync/example-ui/button";
import { Input } from "@pylonsync/example-ui/input";
import { Label } from "@pylonsync/example-ui/label";
import { useAuth } from "./MarketProvider";
import { DEMO } from "./market";

// The sign-in surface shown wherever a write needs a real user (sell, make an
// offer, manage your market). Email/password, no verification email. Prefilled
// with the seeded demo account so it's one click to a working session.
export function LoginCard({
  title = "Sign in to continue",
  blurb = "Real email/password auth — no verification email. The demo account is prefilled; just hit Log in.",
}: {
  title?: string;
  blurb?: string;
}) {
  const { signIn, signUp } = useAuth();
  const [mode, setMode] = useState<"login" | "register">("login");
  // Prefill the demo credentials so a reviewer can sign in instantly.
  const [email, setEmail] = useState(DEMO.email);
  const [password, setPassword] = useState(DEMO.password);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setErr(null);
    try {
      if (mode === "login") await signIn(email, password);
      else await signUp(email, password, name);
    } catch (e) {
      setErr((e as Error).message ?? "Auth failed");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mx-auto max-w-sm space-y-4 rounded-xl border bg-card p-6">
      <div>
        <h2 className="text-lg font-semibold tracking-tight">{title}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{blurb}</p>
      </div>
      <form onSubmit={submit} className="space-y-3">
        {mode === "register" ? (
          <div className="space-y-1.5">
            <Label htmlFor="lc-name">Name</Label>
            <Input
              id="lc-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Pat Pylon"
            />
          </div>
        ) : null}
        <div className="space-y-1.5">
          <Label htmlFor="lc-email">Email</Label>
          <Input
            id="lc-email"
            type="email"
            required
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            placeholder="you@example.com"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="lc-password">Password</Label>
          <Input
            id="lc-password"
            type="password"
            required
            minLength={8}
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder={mode === "register" ? "8+ characters" : ""}
          />
        </div>
        {err ? <p className="text-sm text-destructive">{err}</p> : null}
        <Button type="submit" disabled={busy} className="w-full">
          {busy
            ? "…"
            : mode === "login"
              ? "Log in"
              : "Create account"}
        </Button>
      </form>
      <p className="text-center text-xs text-muted-foreground">
        {mode === "login" ? (
          <>
            No account?{" "}
            <button
              type="button"
              className="font-medium text-foreground hover:underline"
              onClick={() => {
                setMode("register");
                setEmail("");
                setPassword("");
                setErr(null);
              }}
            >
              Sign up
            </button>
          </>
        ) : (
          <>
            Have an account?{" "}
            <button
              type="button"
              className="font-medium text-foreground hover:underline"
              onClick={() => {
                setMode("login");
                setEmail(DEMO.email);
                setPassword(DEMO.password);
                setErr(null);
              }}
            >
              Log in
            </button>
          </>
        )}
      </p>
    </div>
  );
}
