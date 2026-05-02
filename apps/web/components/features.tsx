type Lane = "app" | "game" | "both";

const FEATURES: { lane: Lane; title: string; desc: string }[] = [
  {
    lane: "app",
    title: "Typed schema",
    desc: "Declare entities with field.string/int/float/boolean/datetime/richtext/id and composite indexes in TypeScript. Migrations apply on save.",
  },
  {
    lane: "app",
    title: "Live queries",
    desc: "db.useQuery is a WebSocket subscription. Pylon walks the change log on every write and pushes the diff. No polling, no cache invalidation.",
  },
  {
    lane: "app",
    title: "Server functions",
    desc: "Queries, mutations, and actions in TypeScript with v.* validators. Filename = RPC name. Call from React with callFn or a typed client.",
  },
  {
    lane: "app",
    title: "Row-level policies",
    desc: "Access rules as string expressions (auth.userId == data.authorId) that live next to the schema. Evaluated in the hot path, compiled to bytecode.",
  },
  {
    lane: "app",
    title: "Auth, included",
    desc: "Magic-link email, OAuth (Google / GitHub / Apple), guest sessions, API keys. No separate service, no Auth0 line-item.",
  },
  {
    lane: "app",
    title: "SQLite or Postgres",
    desc: "SQLite is the default — one file, zero setup. Set DATABASE_URL=postgres://… and the same schema targets Postgres. Nothing else changes.",
  },
  {
    lane: "app",
    title: "Admin studio",
    desc: "Browse tables, inspect live queries, tail logs, and run ad-hoc mutations at /studio. Works against any Pylon deployment; admin-gated in prod.",
  },
  {
    lane: "app",
    title: "File uploads",
    desc: "Presigned uploads out of the box. Files land on local disk or any S3-compatible bucket (R2, Backblaze, MinIO) via one env var.",
  },
  {
    lane: "app",
    title: "Faceted search",
    desc: "Add `search:` to an entity, get BM25 + live facets + sort across millions of rows. Index lives in the same DB, maintained in the same transaction as your writes — no Meilisearch sidecar.",
  },
  {
    lane: "both",
    title: "Durable workflows",
    desc: "Long-running, multi-step workflows with sleep, retries, and event waits. Survive restarts — state checkpointed to storage on every step.",
  },
  {
    lane: "both",
    title: "Background jobs + cron",
    desc: "Enqueue a function to run later with ctx.schedule. Cron entries live in the manifest so the schedule is version-controlled with the code.",
  },
  {
    lane: "game",
    title: "Rooms + presence",
    desc: "WebSocket rooms with per-member presence data, join/leave events, and broadcast. Room state lives in Pylon — no pairing with Ably or Pusher.",
  },
  {
    lane: "game",
    title: "Tick-based shards",
    desc: "Authoritative 20/30/60 tps loops in Rust. Area-of-interest filtering, snapshot + delta replication, late-join, observer slots.",
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
        <h2 className="section-title">One framework, twelve primitives.</h2>
        <p className="section-sub">
          The pieces you usually stitch together ship as one system. Use the app
          side on its own, or layer on realtime, workflows, search, and
          game-shaped primitives when the product needs them.
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
