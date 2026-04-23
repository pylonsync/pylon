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
  { feat: "Declarative schema", pylon: "yes", convex: "yes", supabase: "part", firebase: "part" },
  { feat: "Live queries", pylon: "yes", convex: "yes", supabase: "part", firebase: "yes" },
  { feat: "TypeScript functions", pylon: "yes", convex: "yes", supabase: "part", firebase: "part" },
  { feat: "Self-hosted, one binary", pylon: "yes", convex: "no", supabase: "part", firebase: "no" },
  { feat: "Authoritative game loop", pylon: "yes", convex: "no", supabase: "no", firebase: "no" },
  { feat: "No vendor lock-in", pylon: "yes", convex: "no", supabase: "part", firebase: "no" },
];

const GAME_ROWS: GameRow[] = [
  { feat: "Tick-based authority", pylon: "yes", colyseus: "yes", playroom: "part", nakama: "yes" },
  { feat: "Area-of-interest", pylon: "yes", colyseus: "part", playroom: "no", nakama: "part" },
  { feat: "Matchmaker included", pylon: "yes", colyseus: "part", playroom: "yes", nakama: "yes" },
  { feat: "Declarative app data", pylon: "yes", colyseus: "no", playroom: "no", nakama: "part" },
  { feat: "Live queries for UI", pylon: "yes", colyseus: "no", playroom: "no", nakama: "no" },
  { feat: "Self-hosted, one binary", pylon: "yes", colyseus: "yes", playroom: "no", nakama: "yes" },
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
