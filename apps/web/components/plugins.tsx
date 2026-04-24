type PluginGroup = {
  name: string;
  blurb: string;
  plugins: { name: string; desc: string }[];
};

const GROUPS: PluginGroup[] = [
  {
    name: "Auth & identity",
    blurb: "Ship real auth without wiring a third-party SDK.",
    plugins: [
      { name: "password_auth", desc: "Email + password with bcrypt, rate-limited login, password reset." },
      { name: "jwt", desc: "Sign and verify JWTs; issue access + refresh tokens." },
      { name: "totp", desc: "TOTP enrollment + verification for 2FA." },
      { name: "api_keys", desc: "Scoped keys for service-to-service auth." },
      { name: "organizations", desc: "Orgs, members, roles — ready to compose with policies." },
      { name: "tenant_scope", desc: "Row-scoping helper for multi-tenant apps." },
    ],
  },
  {
    name: "Data behaviors",
    blurb: "Attach to entities in the manifest. No middleware plumbing.",
    plugins: [
      { name: "timestamps", desc: "createdAt + updatedAt fields, maintained on every write." },
      { name: "soft_delete", desc: "Mark rows deleted without losing them; queries filter automatically." },
      { name: "slugify", desc: "Generate URL-safe slugs from a source field." },
      { name: "validation", desc: "Per-field custom validators that run before writes commit." },
      { name: "computed", desc: "Derived fields recomputed when dependencies change." },
      { name: "versioning", desc: "Optimistic concurrency control + full row history." },
      { name: "cascade", desc: "On-delete cascade for related entities." },
    ],
  },
  {
    name: "Security & limits",
    blurb: "Defense in depth without a reverse-proxy zoo.",
    plugins: [
      { name: "rate_limit", desc: "Per-IP and per-user limits with sliding windows." },
      { name: "csrf", desc: "Origin + Sec-Fetch-Site checks on state-changing routes." },
      { name: "cors", desc: "Origin allowlist, enforced in non-dev mode." },
      { name: "audit_log", desc: "Structured log of every mutation with actor + diff." },
      { name: "net_guard", desc: "SSRF protection for outbound fetches from server functions." },
      { name: "session_expiry", desc: "Configurable session lifetime + idle timeout." },
    ],
  },
  {
    name: "Storage & search",
    blurb: "First-class files, text, and vectors.",
    plugins: [
      { name: "file_storage", desc: "Presigned uploads to local disk or any S3-compatible bucket." },
      { name: "search", desc: "Full-text search across entity fields. No external Elasticsearch." },
      { name: "vector_search", desc: "Embedding storage + cosine / dot-product nearest-neighbor queries." },
    ],
  },
  {
    name: "Integrations",
    blurb: "Talk to the outside world without a worker service.",
    plugins: [
      { name: "stripe", desc: "Billing primitives — customers, subscriptions, webhooks verified." },
      { name: "webhooks", desc: "Outbound webhook delivery with retries + signature signing." },
      { name: "email", desc: "Transactional email via SES, Resend, Postmark, or SMTP." },
      { name: "ai_proxy", desc: "Call OpenAI / Anthropic / local LLMs; streaming + caching + usage metering." },
      { name: "mcp", desc: "Expose your schema + functions as an MCP server for Claude / Cursor / Zed." },
    ],
  },
  {
    name: "Ops",
    blurb: "Run it like production from day one.",
    plugins: [
      { name: "feature_flags", desc: "Boolean + percentage rollouts keyed by user / org / tenant." },
      { name: "cache", desc: "Redis-style GET / SET / TTL; in-memory by default, Redis-compatible on demand." },
      { name: "cache_client", desc: "Client-side read-through helpers against the cache plugin." },
    ],
  },
];

export function Plugins() {
  return (
    <section className="section" id="plugins">
      <div className="container-page">
        <div className="section-label">Batteries included</div>
        <h2 className="section-title">Thirty-one plugins in the binary.</h2>
        <p className="section-sub">
          Every plugin below ships in <code className="mono">pylon</code>. Enable what you need in your manifest, ignore the rest. No npm install, no sidecar container, no "we integrate with X, you bring the X."
        </p>

        <div className="plugin-grid">
          {GROUPS.map((g) => (
            <div className="plugin-group" key={g.name}>
              <div className="plugin-group-head">
                <h3 className="plugin-group-name">{g.name}</h3>
                <p className="plugin-group-blurb">{g.blurb}</p>
              </div>
              <ul className="plugin-list">
                {g.plugins.map((p) => (
                  <li key={p.name} className="plugin-item">
                    <code className="plugin-name">{p.name}</code>
                    <span className="plugin-desc">{p.desc}</span>
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>

        <div className="plugin-hint">
          <code className="mono">pylon plugins list</code> to see everything available ·{" "}
          <code className="mono">pylon plugins info &lt;name&gt;</code> for config details
        </div>
      </div>
    </section>
  );
}
