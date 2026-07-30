"use client";

import React, { useState } from "react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
} from "@/components/ui/card";
import {
  Field,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { useAuth } from "./MarketProvider";
import { DEMO } from "./market";

// The sign-in surface shown wherever a write needs a real user (sell, make an
// offer, manage your market). Email/password, no verification email. Prefilled
// with the seeded demo account so it's one click to a working session.
export function LoginCard({
  title = "Sign in to continue",
  blurb = "Use the ready demo account or create your own. No verification email is required.",
  headingLevel = 2,
}: {
  title?: string;
  blurb?: string;
  headingLevel?: 1 | 2;
}) {
  const { signIn, signUp } = useAuth();
  const [mode, setMode] = useState<"login" | "register">("login");
  // Prefill the demo credentials so a reviewer can sign in instantly.
  const [email, setEmail] = useState<string>(DEMO.email);
  const [password, setPassword] = useState<string>(DEMO.password);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const Heading = headingLevel === 1 ? "h1" : "h2";

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
    <Card className="mx-auto max-w-sm">
      <CardHeader>
        <Heading className="text-xl font-semibold tracking-[-0.02em]">
          {title}
        </Heading>
        <CardDescription className="text-pretty leading-5">
          {blurb}
        </CardDescription>
      </CardHeader>
      <CardContent>
        <form onSubmit={submit}>
          <FieldGroup className="gap-4">
            {mode === "register" ? (
              <Field>
                <FieldLabel htmlFor="lc-name">Name</FieldLabel>
                <Input
                  id="lc-name"
                  name="name"
                  autoComplete="name"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="Pat Pylon"
                />
              </Field>
            ) : null}
            <Field>
              <FieldLabel htmlFor="lc-email">Email</FieldLabel>
              <Input
                id="lc-email"
                name="email"
                type="email"
                required
                autoComplete="email"
                spellCheck={false}
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                placeholder="you@example.com"
              />
            </Field>
            <Field>
              <FieldLabel htmlFor="lc-password">Password</FieldLabel>
              <Input
                id="lc-password"
                name="password"
                type="password"
                required
                minLength={8}
                autoComplete={
                  mode === "login" ? "current-password" : "new-password"
                }
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder={mode === "register" ? "8+ characters" : ""}
              />
            </Field>
            <div aria-live="polite">
              {err ? (
                <Alert variant="destructive">
                  <AlertDescription>{err}</AlertDescription>
                </Alert>
              ) : null}
            </div>
            <Button type="submit" disabled={busy} className="w-full">
              {busy ? <Spinner data-icon="inline-start" /> : null}
              {busy
                ? mode === "login"
                  ? "Logging in…"
                  : "Creating account…"
                : mode === "login"
                  ? "Log in"
                  : "Create account"}
            </Button>
          </FieldGroup>
        </form>
      </CardContent>
      <CardFooter className="justify-center text-xs text-muted-foreground">
        {mode === "login" ? (
          <>
            No account?{" "}
            <Button
              type="button"
              variant="link"
              size="sm"
              className="px-1"
              onClick={() => {
                setMode("register");
                setEmail("");
                setPassword("");
                setErr(null);
              }}
            >
              Sign up
            </Button>
          </>
        ) : (
          <>
            Have an account?{" "}
            <Button
              type="button"
              variant="link"
              size="sm"
              className="px-1"
              onClick={() => {
                setMode("login");
                setEmail(DEMO.email);
                setPassword(DEMO.password);
                setErr(null);
              }}
            >
              Log in
            </Button>
          </>
        )}
      </CardFooter>
    </Card>
  );
}
