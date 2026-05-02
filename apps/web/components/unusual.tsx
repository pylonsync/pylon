import { CodeLines } from "@/lib/highlight";

function UnusualCard1() {
  const before = `BEGIN;
  SELECT balance FROM accts WHERE id = $1;
  UPDATE accts SET balance = balance - $2 ...;
  UPDATE accts SET balance = balance + $2 ...;
  INSERT INTO transfers ...;
COMMIT;`;
  const after = `export const transfer = mutation({
  handler: async (ctx, { from, to, amount }) => {
    const a = await ctx.db.get(from);
    await ctx.db.patch(from, { bal: a.bal - amount });
    await ctx.db.patch(to,   { bal: a.bal + amount });
    await ctx.db.insert("Transfer", { from, to, amount });
  },
});`;

  return (
    <div className="unusual-card">
      <div className="unusual-num">01</div>
      <h3 className="unusual-title">
        The <code>handler</code> is the transaction.
      </h3>
      <p className="unusual-desc">
        No <code className="mono">BEGIN</code>, no <code className="mono">COMMIT</code>, no
        stale-read bugs. The whole mutation runs under serializable isolation and
        auto-retries on conflict.
      </p>
      <div className="unusual-visual">
        <pre style={{ margin: 0, fontFamily: "inherit" }}>
          {before.split("\n").map((ln, i) => (
            <div key={i} className="line-strike">
              {ln || "\u00A0"}
            </div>
          ))}
        </pre>
        <div style={{ height: 10 }} />
        <pre style={{ margin: 0, fontFamily: "inherit" }}>
          <CodeLines code={after} lang="ts" />
        </pre>
      </div>
    </div>
  );
}

function UnusualCard2() {
  const rows: { name: string; tps: string }[] = [
    { name: "canvas", tps: "30 tps" },
    { name: "cursors", tps: "60 tps" },
    { name: "agents", tps: "10 tps" },
    { name: "match", tps: "20 tps" },
    { name: "mmo_zone", tps: "15 tps" },
  ];
  return (
    <div className="unusual-card">
      <div className="unusual-num">02</div>
      <h3 className="unusual-title">
        Tick-based shards for anything with a loop.
      </h3>
      <p className="unusual-desc">
        Not just games. A collaborative canvas is a 30 tps simulation. Live cursors
        are presence events at 60 tps. An agent swarm is a deterministic step function.
      </p>
      <div className="unusual-visual">
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "1fr auto",
            gap: 6,
            fontSize: 11,
          }}
        >
          {rows.map((r) => (
            <Row key={r.name} name={r.name} tps={r.tps} />
          ))}
        </div>
      </div>
    </div>
  );
}

function Row({ name, tps }: { name: string; tps: string }) {
  return (
    <>
      <span>
        <span className="kw">useShard</span>(<span className="str">&quot;{name}&quot;</span>)
      </span>
      <span className="ok">{tps}</span>
    </>
  );
}

function UnusualCard3() {
  return (
    <div className="unusual-card">
      <div className="unusual-num">03</div>
      <h3 className="unusual-title">Local-first sync with an IndexedDB mirror.</h3>
      <p className="unusual-desc">
        Reads hit a local mirror first; writes queue optimistically and reconcile on
        reconnect. Offline by default, no extra config.
      </p>
      <div className="unusual-visual">
        <div style={{ fontSize: 11, lineHeight: 1.9 }}>
          <div>
            <span className="dim">client  ·</span>{" "}
            <span className="ok">▇▇▇▇▇▇▇▇▇▇▇▇</span>{" "}
            <span className="dim">reads 0.4ms</span>
          </div>
          <div>
            <span className="dim">mirror  ·</span>{" "}
            <span style={{ color: "var(--accent)" }}>▇▇▇▇▇▇▇▇▇</span>
            <span className="dim">▁▁▁</span>{" "}
            <span className="dim">indexeddb</span>
          </div>
          <div>
            <span className="dim">socket  ·</span>{" "}
            <span className="dim">▁▁</span>
            <span className="ok">▇</span>
            <span className="dim">▁▁▁▁▁</span>
            <span className="ok">▇</span>
            <span className="dim">▁▁▁</span>{" "}
            <span className="dim">deltas</span>
          </div>
          <div>
            <span className="dim">server  ·</span>{" "}
            <span style={{ color: "var(--blue)" }}>▇▇▇▇▇▇▇▇▇▇▇▇</span>{" "}
            <span className="dim">source of truth</span>
          </div>
        </div>
        <div
          style={{
            marginTop: 10,
            paddingTop: 8,
            borderTop: "1px solid var(--border)",
            fontSize: 11,
            color: "var(--text-3)",
          }}
        >
          <span className="ok">●</span> offline · writes queued, reads from mirror
        </div>
      </div>
    </div>
  );
}

export function Unusual() {
  return (
    <section className="section" id="unusual">
      <div className="container-page">
        <div className="section-label">The unusual bits</div>
        <h2 className="section-title">Three choices that make it feel cohesive.</h2>
        <p className="section-sub">
          The Rails feeling comes from strong defaults. Pylon brings that same
          bias to transactions, realtime loops, and local-first data.
        </p>
        <div className="unusual-grid">
          <UnusualCard1 />
          <UnusualCard2 />
          <UnusualCard3 />
        </div>
      </div>
    </section>
  );
}
