type Kind = "yes" | "part" | "no";

type AppRow = {
  feat: string;
  pylon: Kind;
  convex: Kind;
  supabase: Kind;
  firebase: Kind;
};

type GameRow = {
  feat: string;
  pylon: Kind;
  colyseus: Kind;
  playroom: Kind;
  nakama: Kind;
};

const APP_ROWS: AppRow[] = [
  // Convex and Supabase both ship declarative schemas. Convex's is TS
  // (`defineSchema`); Supabase's is SQL migrations + RLS — fully declarative
  // in the "single source of truth in your repo" sense. Firebase lacks a
  // schema layer entirely (Firestore is schemaless) — flagged as "part"
  // because rules act as a soft schema.
  { feat: "Declarative schema", pylon: "yes", convex: "yes", supabase: "yes", firebase: "part" },
  // All four ship realtime subscriptions; Supabase's are first-class via
  // their Realtime service, not a partial.
  { feat: "Live queries", pylon: "yes", convex: "yes", supabase: "yes", firebase: "yes" },
  // Convex = pure TS in-repo. Supabase Edge Functions are TS/Deno but
  // deployed separately and don't share the type graph with your client.
  // Firebase Cloud Functions support TS but compile to JS and ship via
  // gcloud — same "deployed separately" caveat.
  { feat: "TypeScript functions", pylon: "yes", convex: "yes", supabase: "part", firebase: "part" },
  // Convex has FTS (no facets). Supabase has Postgres tsvector FTS (no
  // native facets — you build them with GROUP BY queries). Firebase has
  // no FTS at all (docs recommend Algolia).
  { feat: "Native faceted search", pylon: "yes", convex: "no", supabase: "part", firebase: "no" },
  // Convex and Supabase both have self-host paths (docker-compose stacks),
  // so claiming they don't is both wrong and easily falsifiable. The
  // honest differentiator is single-process below.
  { feat: "Self-hosted", pylon: "yes", convex: "yes", supabase: "yes", firebase: "no" },
  { feat: "Single process", pylon: "yes", convex: "no", supabase: "no", firebase: "no" },
  { feat: "Authoritative game loop", pylon: "yes", convex: "no", supabase: "no", firebase: "no" },
  { feat: "Open source", pylon: "yes", convex: "yes", supabase: "yes", firebase: "no" },
];

const GAME_ROWS: GameRow[] = [
  // Colyseus uses setSimulationInterval; Nakama match handlers run at a
  // configurable tick rate. Playroom is closer to relayed state-sync with
  // a "host" client than full server tick authority — flagged as "part".
  { feat: "Tick-based authority", pylon: "yes", colyseus: "yes", playroom: "part", nakama: "yes" },
  // Colyseus offers @filter decorators for selective state sync (manual
  // per-property filtering). Nakama supports selective broadcast via
  // presence lists, not spatial AOI. Both = "part" because they require
  // hand-rolled spatial logic on top.
  { feat: "Area-of-interest", pylon: "yes", colyseus: "part", playroom: "no", nakama: "part" },
  // Colyseus docs: "Built-in Matchmaking. Automatic room creation and
  // player matching. Customizable filtering and sorting." Full feature.
  { feat: "Matchmaker included", pylon: "yes", colyseus: "yes", playroom: "yes", nakama: "yes" },
  // None of the game servers ship a declarative app-data layer. Nakama
  // has a Storage Engine that's halfway there (typed JSON blobs).
  { feat: "Declarative app data", pylon: "yes", colyseus: "no", playroom: "no", nakama: "part" },
  { feat: "Live queries for UI", pylon: "yes", colyseus: "no", playroom: "no", nakama: "no" },
  // Colyseus is "a standard Node.js application" — your code + Node, not
  // a single binary. Nakama is a single Go binary + Postgres dependency.
  // Marking Colyseus "part" to be honest about the deployment shape.
  { feat: "Self-hosted, one binary", pylon: "yes", colyseus: "part", playroom: "no", nakama: "yes" },
];

function Mark({ kind }: { kind: Kind }) {
  if (kind === "yes") return <span className="mark-yes">●</span>;
  if (kind === "part") return <span className="mark-part">◐</span>;
  return <span className="mark-no">○</span>;
}

type Col<R> = { key: keyof R; label: string; self?: boolean };

function Table<R extends { feat: string } & Record<string, unknown>>({
  title,
  tag,
  cols,
  rows,
}: {
  title: string;
  tag: string;
  cols: Col<R>[];
  rows: R[];
}) {
  return (
    <div className="compare-table">
      <div className="compare-title-row">
        <div className="compare-title">
          {title}
          <span className="tag">{tag}</span>
        </div>
      </div>
      <div className="compare-row head">
        <div className="col-label">Feature</div>
        {cols.map((c) => (
          <div
            key={String(c.key)}
            className="col col-label"
            style={c.self ? { color: "var(--accent)" } : undefined}
          >
            {c.label}
          </div>
        ))}
      </div>
      {rows.map((r, i) => (
        <div className="compare-row" key={i}>
          <div className="col-feat">{r.feat}</div>
          {cols.map((c) => (
            <div key={String(c.key)} className={`col ${c.self ? "self" : ""}`}>
              <Mark kind={r[c.key] as Kind} />
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}

export function Compare() {
  const appCols: Col<AppRow>[] = [
    { key: "pylon", label: "pylon", self: true },
    { key: "convex", label: "Convex" },
    { key: "supabase", label: "Supabase" },
    { key: "firebase", label: "Firebase" },
  ];
  const gameCols: Col<GameRow>[] = [
    { key: "pylon", label: "pylon", self: true },
    { key: "colyseus", label: "Colyseus" },
    { key: "playroom", label: "Playroom" },
    { key: "nakama", label: "Nakama" },
  ];

  return (
    <section className="section" id="compare">
      <div className="container-page">
        <div className="section-label">Compare</div>
        <h2 className="section-title">
          The only option that does both well — in one process.
        </h2>
        <p className="section-sub">
          Pick an app backend <em>or</em> a game server and you&apos;ll stitch the other
          in. pylon ships with both primitives, sharing auth, storage, and policies.
        </p>

        <div className="compare-grid">
          <Table
            title="For apps"
            tag="vs. app-backend stacks"
            cols={appCols}
            rows={APP_ROWS}
          />
          <Table
            title="For games"
            tag="vs. game servers"
            cols={gameCols}
            rows={GAME_ROWS}
          />
        </div>

        <div
          style={{
            marginTop: 20,
            display: "flex",
            gap: 20,
            flexWrap: "wrap",
            fontFamily: "var(--font-mono)",
            fontSize: 11.5,
            color: "var(--text-3)",
          }}
        >
          <span>
            <span className="mark-yes" style={{ color: "var(--green)" }}>
              ●
            </span>{" "}
            first-class
          </span>
          <span>
            <span className="mark-part" style={{ color: "var(--accent)" }}>
              ◐
            </span>{" "}
            partial / via extension
          </span>
          <span>
            <span className="mark-no" style={{ color: "var(--text-4)" }}>
              ○
            </span>{" "}
            not supported
          </span>
        </div>
      </div>
    </section>
  );
}
