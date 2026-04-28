import { createFileRoute, Link } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { getMe, type Me } from "../lib/pylon";

export const Route = createFileRoute("/")({
  component: Home,
});

function Home() {
  const [me, setMe] = useState<Me | null | undefined>(undefined);

  useEffect(() => {
    getMe().then(setMe);
  }, []);

  return (
    <main style={{ maxWidth: 720, margin: "5rem auto", padding: "0 1.5rem" }}>
      <h1 style={{ fontSize: "2.5rem", margin: 0 }}>__APP_NAME__</h1>
      <p style={{ color: "#666", fontSize: "1.125rem", marginTop: "0.5rem" }}>
        A Pylon app. Backend on <code>:4321</code>, TanStack Start on <code>:3000</code>.
      </p>

      <div style={{ marginTop: "3rem", display: "flex", gap: "1rem" }}>
        {me === undefined ? (
          <span style={{ color: "#999" }}>Loading…</span>
        ) : me ? (
          <Link to="/dashboard" style={primaryBtn}>Open dashboard →</Link>
        ) : (
          <Link to="/login" style={primaryBtn}>Sign in →</Link>
        )}
        <a href="http://localhost:4321/studio" style={secondaryBtn}>
          Open Studio
        </a>
      </div>

      <hr style={{ margin: "4rem 0 2rem", border: "none", borderTop: "1px solid #eee" }} />

      <h2 style={{ fontSize: "1.25rem" }}>What's wired up</h2>
      <ul style={{ lineHeight: 1.8, color: "#444" }}>
        <li>
          <strong>File-based routing</strong> via TanStack Start —{" "}
          <code>app/routes/*</code> auto-generate the route tree.
        </li>
        <li>
          <strong>Magic-code auth</strong> at <code>/login</code>.
        </li>
        <li>
          <strong>Auth-gated dashboard</strong> at <code>/dashboard</code> with{" "}
          <code>beforeLoad</code> redirecting unauthenticated visits.
        </li>
        <li>
          <strong>Same-origin API proxy</strong> via <code>app.config.ts</code> — no CORS needed.
        </li>
      </ul>
    </main>
  );
}

const primaryBtn: React.CSSProperties = {
  padding: "0.75rem 1.5rem",
  background: "#111",
  color: "white",
  borderRadius: 6,
  textDecoration: "none",
  fontWeight: 500,
};
const secondaryBtn: React.CSSProperties = {
  padding: "0.75rem 1.5rem",
  background: "white",
  color: "#111",
  border: "1px solid #ddd",
  borderRadius: 6,
  textDecoration: "none",
  fontWeight: 500,
};
