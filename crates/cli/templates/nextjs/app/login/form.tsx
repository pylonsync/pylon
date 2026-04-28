"use client";

import { useState, useTransition } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { startMagicCode, verifyMagicCode } from "./actions";

export function LoginForm({ next }: { next?: string }) {
  const [email, setEmail] = useState("");
  const [code, setCode] = useState("");
  const [stage, setStage] = useState<"email" | "code">("email");
  const [devCode, setDevCode] = useState<string | undefined>();
  const [error, setError] = useState<string | null>(null);
  const [pending, startTransition] = useTransition();

  function handleSendCode(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    startTransition(async () => {
      try {
        const { devCode } = await startMagicCode(email);
        setDevCode(devCode);
        setStage("code");
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      }
    });
  }

  if (stage === "email") {
    return (
      <form onSubmit={handleSendCode} className="grid gap-4">
        <div className="grid gap-2">
          <Label htmlFor="email">Email</Label>
          <Input
            id="email"
            type="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            required
            autoComplete="email"
            autoFocus
            placeholder="alice@example.com"
          />
        </div>
        {error && (
          <p className="rounded bg-destructive/10 px-3 py-2 text-sm text-destructive">{error}</p>
        )}
        <Button type="submit" disabled={pending || !email}>
          {pending ? "Sending…" : "Send code"}
        </Button>
      </form>
    );
  }

  return (
    <form action={verifyMagicCode} className="grid gap-4">
      <input type="hidden" name="email" value={email} />
      {next && <input type="hidden" name="next" value={next} />}
      <div className="grid gap-2">
        <Label htmlFor="code">
          Code sent to <span className="font-semibold">{email}</span>
        </Label>
        <Input
          id="code"
          name="code"
          value={code}
          onChange={(e) => setCode(e.target.value)}
          required
          autoFocus
          autoComplete="one-time-code"
          inputMode="numeric"
          pattern="[0-9]{6}"
          placeholder="123456"
          className="text-center text-xl tracking-[0.4em]"
        />
      </div>
      {devCode && (
        <p className="rounded bg-yellow-50 px-3 py-2 text-sm text-yellow-900 dark:bg-yellow-950 dark:text-yellow-100">
          Dev mode: code is <code className="font-semibold">{devCode}</code>
        </p>
      )}
      <Button type="submit">Verify and sign in</Button>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        onClick={() => {
          setStage("email");
          setCode("");
          setDevCode(undefined);
        }}
        className="justify-start"
      >
        ← Use a different email
      </Button>
    </form>
  );
}
