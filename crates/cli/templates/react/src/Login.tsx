import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { pylonJson, setToken, type Me } from "@/lib/pylon";

export function Login({ onSignedIn }: { onSignedIn: (me: Me) => void }) {
  const [email, setEmail] = useState("");
  const [code, setCode] = useState("");
  const [stage, setStage] = useState<"email" | "code">("email");
  const [devCode, setDevCode] = useState<string | undefined>();
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  async function handleSendCode(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setLoading(true);
    try {
      const resp = await pylonJson<{ sent: boolean; dev_code?: string }>(
        "/api/auth/magic/send",
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ email }),
        },
      );
      setDevCode(resp.dev_code);
      setStage("code");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }

  async function handleVerify(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setLoading(true);
    try {
      const resp = await pylonJson<{
        token: string;
        user_id: string;
        expires_at: number;
      }>("/api/auth/magic/verify", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ email, code }),
      });
      setToken(resp.token);
      const me = await pylonJson<Me>("/api/auth/me");
      onSignedIn(me);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }

  return (
    <main className="mx-auto max-w-md px-6 py-16">
      <Card>
        <CardHeader>
          <CardTitle>Sign in</CardTitle>
          <CardDescription>We'll email you a 6-digit code.</CardDescription>
        </CardHeader>
        <CardContent>
          {stage === "email" ? (
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
                <p className="rounded bg-destructive/10 px-3 py-2 text-sm text-destructive">
                  {error}
                </p>
              )}
              <Button type="submit" disabled={loading || !email}>
                {loading ? "Sending…" : "Send code"}
              </Button>
            </form>
          ) : (
            <form onSubmit={handleVerify} className="grid gap-4">
              <div className="grid gap-2">
                <Label htmlFor="code">
                  Code sent to <span className="font-semibold">{email}</span>
                </Label>
                <Input
                  id="code"
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
              {error && (
                <p className="rounded bg-destructive/10 px-3 py-2 text-sm text-destructive">
                  {error}
                </p>
              )}
              <Button type="submit" disabled={loading}>
                {loading ? "Verifying…" : "Verify and sign in"}
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => {
                  setStage("email");
                  setCode("");
                  setDevCode(undefined);
                  setError(null);
                }}
                className="justify-start"
              >
                ← Use a different email
              </Button>
            </form>
          )}
          <p className="mt-6 text-xs text-muted-foreground">
            In dev mode the code prints to the Pylon server's stdout, and appears below the form
            once requested.
          </p>
        </CardContent>
      </Card>
    </main>
  );
}
