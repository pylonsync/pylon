type Lane = "app" | "game" | "both";

const FEATURES: { lane: Lane; title: string; desc: string }[] = [
  {
    lane: "app",
    title: "Declarative schema",
    desc: "JSON schema with refs, indexes, and per-field validators. Migrations auto-generated.",
  },
  {
    lane: "app",
    title: "Real-time sync",
    desc: "Queries are WebSocket subscriptions by default. Subsecond fan-out, no polling.",
  },
  {
    lane: "app",
    title: "TypeScript functions",
    desc: "Queries and mutations run server-side with Zod validators. Types flow to the client.",
  },
  {
    lane: "app",
    title: "Auth, built-in",
    desc: "Magic codes, OAuth (Google, GitHub, Apple), sessions, row-level policies — no separate service.",
  },
  {
    lane: "game",
    title: "Tick-based shards",
    desc: "Authoritative 20/30/60 tps loops in Rust. Deterministic. Snapshot + delta replication.",
  },
  {
    lane: "game",
    title: "Matchmaker + AoI",
    desc: "Room-based matchmaking, area-of-interest filtering, backfill, late-join, observer slots.",
  },
  {
    lane: "both",
    title: "Jobs & workflows",
    desc: "Durable background jobs, cron, and multi-step workflows with retries. Survive restarts.",
  },
  {
    lane: "both",
    title: "Policies",
    desc: "Row-level policies as code. One rule enforces reads, mutations, and shard inputs.",
  },
];

function LaneLabel({ lane }: { lane: Lane }) {
  const map = { app: "App", game: "Game", both: "App + Game" } as const;
  return <div className={`feature-lane ${lane}`}>{map[lane]}</div>;
}

export function Features() {
  return (
    <section className="section" id="features">
      <div className="container-page">
        <div className="section-label">What you get</div>
        <h2 className="section-title">Two lanes, one binary.</h2>
        <p className="section-sub">
          Everything below ships in <code className="mono">statecraft</code>. No sidecars.
          No extra Redis. No separate realtime layer. Either lane is useful alone; together they&apos;re rare.
        </p>

        <div className="features-grid">
          {FEATURES.map((f, i) => (
            <div className="feature-card" key={i}>
              <LaneLabel lane={f.lane} />
              <h3 className="feature-title">{f.title}</h3>
              <p className="feature-desc">{f.desc}</p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
