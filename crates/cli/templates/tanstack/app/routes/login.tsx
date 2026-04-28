import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { pylonJson, setToken } from "../lib/pylon";

export const Route = createFileRoute("/login")({
  component: LoginPage,
});

function LoginPage() {
  const navigate = useNavigate();
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
      navigate({ to: "/dashboard" });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }

  return (
    <main style={cardStyle}>
      <h1 style={{ marginTop: 0 }}>Sign in</h1>
      <p style={{ color: "#666", marginTop: "-0.25rem" }}>
        We'll email you a 6-digit code.
      </p>

      {stage === "email" ? (
        <form onSubmit={handleSendCode} style={formStyle}>
          <label style={labelStyle}>Email</label>
          <input
            type="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            required
            autoComplete="email"
            autoFocus
            style={inputStyle}
          />
          {error && <p style={errorStyle}>{error}</p>}
          <button type="submit" disabled={loading || !email} style={buttonStyle(loading)}>
            {loading ? "Sending…" : "Send code"}
          </button>
        </form>
      ) : (
        <form onSubmit={handleVerify} style={formStyle}>
          <label style={labelStyle}>
            Code sent to <strong>{email}</strong>
          </label>
          <input
            value={code}
            onChange={(e) => setCode(e.target.value)}
            required
            autoFocus
            autoComplete="one-time-code"
            inputMode="numeric"
            pattern="[0-9]{6}"
            placeholder="123456"
            style={{
              ...inputStyle,
              fontSize: "1.25rem",
              letterSpacing: "0.25rem",
              textAlign: "center",
            }}
          />
          {devCode && (
            <p style={devCodeStyle}>
              Dev mode: code is <code style={{ fontWeight: 600 }}>{devCode}</code>
            </p>
          )}
          {error && <p style={errorStyle}>{error}</p>}
          <button type="submit" disabled={loading} style={buttonStyle(loading)}>
            {loading ? "Verifying…" : "Verify and sign in"}
          </button>
          <button
            type="button"
            onClick={() => { setStage("email"); setCode(""); setDevCode(undefined); setError(null); }}
            style={linkButtonStyle}
          >
            ← Use a different email
          </button>
        </form>
      )}
    </main>
  );
}

const cardStyle: React.CSSProperties = {
  maxWidth: 420,
  margin: "5rem auto",
  padding: "2rem",
  background: "white",
  borderRadius: 12,
  border: "1px solid #eee",
};
const formStyle: React.CSSProperties = { display: "grid", gap: "0.75rem" };
const labelStyle: React.CSSProperties = { fontSize: 14, color: "#444" };
const inputStyle: React.CSSProperties = {
  padding: "0.625rem 0.75rem",
  border: "1px solid #ddd",
  borderRadius: 6,
  fontSize: "1rem",
  fontFamily: "inherit",
};
const errorStyle: React.CSSProperties = {
  background: "#fef2f2",
  color: "#991b1b",
  padding: "0.5rem 0.75rem",
  borderRadius: 4,
  fontSize: 13,
  margin: 0,
};
const devCodeStyle: React.CSSProperties = {
  background: "#fef3c7",
  color: "#92400e",
  padding: "0.5rem 0.75rem",
  borderRadius: 4,
  fontSize: 13,
  margin: 0,
};
const linkButtonStyle: React.CSSProperties = {
  background: "transparent",
  border: "none",
  color: "#666",
  fontSize: 13,
  cursor: "pointer",
  padding: 0,
  textAlign: "left",
};
function buttonStyle(disabled: boolean): React.CSSProperties {
  return {
    padding: "0.75rem 1rem",
    background: disabled ? "#ccc" : "#111",
    color: "white",
    border: "none",
    borderRadius: 6,
    fontSize: "1rem",
    fontWeight: 500,
    cursor: disabled ? "not-allowed" : "pointer",
  };
}
