"use client";

import * as React from "react";
import { CodeLines } from "@/lib/highlight";

function InstallCmd() {
  const [copied, setCopied] = React.useState(false);
  const onClick = () => {
    const text = "cargo install pylon-cli";
    navigator.clipboard?.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 1400);
  };
  return (
    <button
      className={`install-cmd ${copied ? "copied" : ""}`}
      onClick={onClick}
      aria-label="Copy install command"
    >
      <span className="dollar">$</span>
      <span>cargo install pylon-cli</span>
      <svg
        className="copy-ico"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.75"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
        <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
      </svg>
      <span className="copied-label">copied</span>
    </button>
  );
}

function GithubButton() {
  return (
    <a
      className="inline-flex items-center gap-2 h-9 px-3.5 rounded-md text-[13px] font-medium border border-[color:var(--border-2)] text-[color:var(--text)] hover:bg-[color:var(--bg-2)] hover:border-[#33333a] transition-colors"
      href="#github"
    >
      <svg
        viewBox="0 0 24 24"
        fill="currentColor"
        style={{ width: 14, height: 14 }}
      >
        <path d="M12 .5C5.65.5.5 5.65.5 12a11.5 11.5 0 0 0 7.86 10.92c.58.1.79-.25.79-.56v-2c-3.2.7-3.88-1.37-3.88-1.37-.52-1.33-1.28-1.69-1.28-1.69-1.05-.72.08-.7.08-.7 1.16.08 1.77 1.2 1.77 1.2 1.03 1.77 2.7 1.26 3.36.96.1-.75.4-1.26.73-1.55-2.55-.29-5.24-1.28-5.24-5.7 0-1.26.45-2.3 1.19-3.11-.12-.3-.52-1.49.12-3.1 0 0 .97-.31 3.18 1.18a11 11 0 0 1 5.78 0c2.21-1.49 3.18-1.18 3.18-1.18.64 1.61.24 2.8.12 3.1.74.81 1.19 1.85 1.19 3.11 0 4.43-2.7 5.41-5.27 5.69.41.36.78 1.06.78 2.15v3.19c0 .31.21.67.8.56A11.5 11.5 0 0 0 23.5 12C23.5 5.65 18.35.5 12 .5z" />
      </svg>
      View on GitHub
      <span className="text-dim text-mono" style={{ fontSize: 11, marginLeft: 4 }}>
        ★ 2.4k
      </span>
    </a>
  );
}

const SEED_MESSAGES = [
  { name: "maya", color: "#F5B946", text: "shipping the v0.8 tick loop tonight" },
  { name: "jonas", color: "#7AB7FF", text: "pulled it — tests pass on my laptop" },
  { name: "rhea", color: "#5EE6A6", text: "useQuery fires in 4ms here, sub is instant" },
  { name: "maya", color: "#F5B946", text: "cranking area-of-interest up to 150m" },
  { name: "dani", color: "#C89DFF", text: "so the shard handles 600 entities?" },
  { name: "jonas", color: "#7AB7FF", text: "✓ green on staging" },
  { name: "rhea", color: "#5EE6A6", text: "merging now, pushing to Workers" },
  { name: "maya", color: "#F5B946", text: "love that we dropped 3 services for this" },
];

function ChatDemo() {
  const [count, setCount] = React.useState(3);
  React.useEffect(() => {
    const t = setInterval(() => {
      setCount((c) => (c >= SEED_MESSAGES.length ? 3 : c + 1));
    }, 2400);
    return () => clearInterval(t);
  }, []);
  const visible = SEED_MESSAGES.slice(Math.max(0, count - 3), count);
  const now = (i: number) => `${i * 2 + 1}m`;

  return (
    <div className="chat-app">
      <div className="chat-header">
        <div className="chat-header-title">
          <span className="text-dim text-mono" style={{ fontSize: 11 }}>
            #
          </span>
          engineering
        </div>
        <div className="chat-presence">
          {["M", "J", "R", "D"].map((c, i) => (
            <div
              key={i}
              className="presence-dot"
              style={{ background: ["#F5B946", "#7AB7FF", "#5EE6A6", "#C89DFF"][i] }}
            >
              {c}
            </div>
          ))}
        </div>
      </div>
      <div className="chat-messages">
        {visible.map((m, i) => (
          <div key={`${count}-${i}`} className="chat-msg">
            <div className="chat-avatar" style={{ background: m.color }}>
              {m.name[0].toUpperCase()}
            </div>
            <div className="chat-msg-body">
              <div className="chat-msg-meta">
                <span className="chat-msg-name">{m.name}</span>
                <span>{now(visible.length - i - 1)}</span>
              </div>
              <div className="chat-msg-text">{m.text}</div>
            </div>
          </div>
        ))}
      </div>
      <div className="chat-input">
        <span>&gt;</span>
        <span>type a message</span>
        <span className="typing-caret" />
      </div>
    </div>
  );
}

function HeroTerminal() {
  const lines = [
    { t: "prompt", v: "❯ ", c: "cargo install pylon-cli" },
    { t: "out", v: "   Compiling pylon-cli v0.8.2" },
    { t: "out-ok", v: "    Finished release in 41.2s" },
    { t: "prompt", v: "❯ ", c: "pylon dev" },
    { t: "out", v: "  schema  · 12 tables loaded" },
    { t: "out", v: "  shards  · 4 rooms · area-of-interest: 64m" },
    { t: "out-accent", v: "  serving on http://localhost:4242" },
    { t: "out-ok", v: "  ✓ hot-reload · type-safe client regenerated" },
  ] as const;
  return (
    <div className="terminal">
      {lines.map((l, i) => (
        <div className="line" key={i}>
          {l.t === "prompt" && (
            <>
              <span className="accent">{l.v}</span>
              <span className="cmd">{l.c}</span>
            </>
          )}
          {l.t === "out" && <span className="dim">{l.v}</span>}
          {l.t === "out-ok" && <span className="ok">{l.v}</span>}
          {l.t === "out-accent" && <span className="accent">{l.v}</span>}
        </div>
      ))}
    </div>
  );
}

type Entity = {
  x: number;
  y: number;
  tx: number;
  ty: number;
  c: string;
  r: number;
  label: string | null;
  history: [number, number][];
};

function GameCanvas() {
  const canvasRef = React.useRef<HTMLCanvasElement | null>(null);

  React.useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const dpr = window.devicePixelRatio || 1;

    const fit = () => {
      const rect = canvas.getBoundingClientRect();
      canvas.width = rect.width * dpr;
      canvas.height = rect.height * dpr;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    };
    fit();
    const ro = new ResizeObserver(fit);
    ro.observe(canvas);

    const N = 7;
    const colors = [
      "#F5B946",
      "#7AB7FF",
      "#5EE6A6",
      "#C89DFF",
      "#F5B946",
      "#7AB7FF",
      "#5EE6A6",
    ];
    const entities: Entity[] = Array.from({ length: N }, (_, i) => ({
      x: Math.random(),
      y: Math.random(),
      tx: Math.random(),
      ty: Math.random(),
      c: colors[i % colors.length],
      r: i === 0 ? 6 : 4.5,
      label: i === 0 ? "P1" : null,
      history: [],
    }));

    const start = performance.now();
    let rafId = 0;
    const tick = (t: number) => {
      const rect = canvas.getBoundingClientRect();
      const W = rect.width;
      const H = rect.height;

      ctx.clearRect(0, 0, W, H);

      ctx.strokeStyle = "rgba(255,255,255,0.035)";
      ctx.lineWidth = 1;
      const gs = 28;
      for (let x = 0; x < W; x += gs) {
        ctx.beginPath();
        ctx.moveTo(x, 0);
        ctx.lineTo(x, H);
        ctx.stroke();
      }
      for (let y = 0; y < H; y += gs) {
        ctx.beginPath();
        ctx.moveTo(0, y);
        ctx.lineTo(W, y);
        ctx.stroke();
      }

      const p1 = entities[0];
      const p1x = p1.x * W;
      const p1y = p1.y * H;
      ctx.strokeStyle = "rgba(122, 183, 255, 0.22)";
      ctx.setLineDash([3, 3]);
      ctx.beginPath();
      ctx.arc(p1x, p1y, 70, 0, Math.PI * 2);
      ctx.stroke();
      ctx.setLineDash([]);

      for (const e of entities) {
        const dx = e.tx - e.x;
        const dy = e.ty - e.y;
        const d = Math.hypot(dx, dy);
        if (d < 0.02) {
          e.tx = Math.random();
          e.ty = Math.random();
        } else {
          const sp = 0.0038;
          e.x += (dx / d) * sp;
          e.y += (dy / d) * sp;
        }
        e.history.push([e.x, e.y]);
        if (e.history.length > 20) e.history.shift();
      }

      for (const e of entities) {
        for (let i = 0; i < e.history.length - 1; i++) {
          const [ax, ay] = e.history[i];
          const [bx, by] = e.history[i + 1];
          const alpha = Math.floor((i / e.history.length) * 40)
            .toString(16)
            .padStart(2, "0");
          ctx.strokeStyle = e.c + alpha;
          ctx.lineWidth = 1;
          ctx.beginPath();
          ctx.moveTo(ax * W, ay * H);
          ctx.lineTo(bx * W, by * H);
          ctx.stroke();
        }
      }

      for (const e of entities) {
        ctx.fillStyle = e.c;
        ctx.beginPath();
        ctx.arc(e.x * W, e.y * H, e.r, 0, Math.PI * 2);
        ctx.fill();
        if (e.label) {
          ctx.fillStyle = "#1a1208";
          ctx.font = "600 8px Geist Mono, monospace";
          ctx.textAlign = "center";
          ctx.textBaseline = "middle";
          ctx.fillText(e.label, e.x * W, e.y * H + 0.5);
        }
      }

      const ticks = Math.floor((t - start) / 50);
      ctx.fillStyle = "#6A6A72";
      ctx.font = "10px Geist Mono, monospace";
      ctx.textAlign = "left";
      ctx.textBaseline = "top";
      ctx.fillText(
        `tick ${ticks.toString().padStart(5, "0")}   ·   20 tps   ·   7 entities`,
        10,
        10,
      );
      ctx.textAlign = "right";
      ctx.fillStyle = "#7AB7FF";
      ctx.fillText("match_1", W - 10, 10);

      rafId = requestAnimationFrame(tick);
    };
    rafId = requestAnimationFrame(tick);

    return () => {
      cancelAnimationFrame(rafId);
      ro.disconnect();
    };
  }, []);

  return <canvas className="game-canvas" ref={canvasRef} />;
}

function GameSnippet() {
  const code = `const { state, send } = useShard("match_1");

state.entities.forEach(e => {
  draw(e.x, e.y, e.color);
});

onKeyDown("ArrowUp", () =>
  send({ move: "up" })
);`;
  return (
    <div className="game-snippet">
      <pre style={{ margin: 0, fontFamily: "inherit" }}>
        <CodeLines code={code} lang="ts" />
      </pre>
    </div>
  );
}

type DemoTab = "chat" | "dashboard" | "shard" | "dev";

function HeroDemo() {
  const [tab, setTab] = React.useState<DemoTab>("dashboard");

  const meta: Record<DemoTab, { file: string; hook: string; status: string }> = {
    dashboard: { file: "apps/admin/Dashboard.tsx", hook: "useAggregate", status: "live" },
    chat: { file: "apps/chat/App.tsx", hook: "useQuery", status: "live" },
    shard: { file: "shards/match.rs", hook: "useShard", status: "20 tps" },
    dev: { file: "~/pylon", hook: "pylon dev", status: "ready" },
  };

  const tabs: { id: DemoTab; num: string; label: string; sub: string }[] = [
    { id: "dashboard", num: "01", label: "Live dashboard", sub: "useAggregate" },
    { id: "chat", num: "02", label: "Realtime chat", sub: "useQuery" },
    { id: "shard", num: "03", label: "Game shard", sub: "useShard" },
    { id: "dev", num: "04", label: "Dev server", sub: "pylon dev" },
  ];

  return (
    <div className="hero-demo">
      <div className="panel">
        <div className="hero-demo-tabs">
          {tabs.map((t) => (
            <button
              key={t.id}
              className={`hero-demo-tab ${tab === t.id ? "active" : ""}`}
              onClick={() => setTab(t.id)}
            >
              <span className="hero-demo-tab-num">{t.num}</span>
              <span className="hero-demo-tab-label">{t.label}</span>
              <span className="hero-demo-tab-sub">{t.sub}</span>
            </button>
          ))}
        </div>

        <div className="panel-header">
          <div className="panel-header-left">
            <span className="panel-dot" />
            <span>{meta[tab].file}</span>
          </div>
          <div className="panel-header-right">
            <span className="text-dim">{meta[tab].hook}</span>
            <span className="text-dim">·</span>
            <span className="text-accent">{meta[tab].status}</span>
          </div>
        </div>

        <div className="hero-demo-body">
          {tab === "chat" && <ChatDemoFull />}
          {tab === "dashboard" && <DashboardDemoFull />}
          {tab === "shard" && <ShardDemoFull />}
          {tab === "dev" && <DevDemoFull />}
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// 3-panel Convex-style demo — Code (with schema tab) | App UI | Data table
// ---------------------------------------------------------------------------

function CodeStack({
  files,
}: {
  files: { name: string; lang: string; langLabel: string; code: string }[];
}) {
  const [active, setActive] = React.useState(0);
  const f = files[active];
  return (
    <div className="code-stack">
      <div className="code-stack-tabs">
        {files.map((file, i) => (
          <button
            key={file.name}
            className={`code-stack-tab ${active === i ? "active" : ""}`}
            onClick={() => setActive(i)}
          >
            <span className="code-stack-tab-lang">{file.langLabel}</span>
            <span>{file.name}</span>
          </button>
        ))}
      </div>
      <div className="code-stack-body">
        <pre style={{ margin: 0, fontFamily: "inherit" }}>
          <CodeLines code={f.code} lang={f.lang} />
        </pre>
      </div>
    </div>
  );
}

function DataTable({
  title,
  tag,
  columns,
  rows,
}: {
  title: string;
  tag: string;
  columns: string[];
  rows: (string | React.ReactNode)[][];
}) {
  return (
    <div className="data-table">
      <div className="data-table-head">
        <span className="data-table-title">{title}</span>
        <span className="data-table-sub">{tag}</span>
      </div>
      <div className="data-table-scroll">
        <table className="data-table-grid">
          <thead>
            <tr>
              {columns.map((c) => (
                <th key={c}>{c}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((row, i) => (
              <tr key={i}>
                {row.map((cell, j) => (
                  <td key={j}>{cell}</td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function ThreePanel({
  code,
  appPanel,
  tablePanel,
}: {
  code: React.ReactNode;
  appPanel: React.ReactNode;
  tablePanel: React.ReactNode;
}) {
  return (
    <div className="three-panel">
      <div className="three-panel-code">{code}</div>
      <div className="three-panel-right">
        <div className="three-panel-app">{appPanel}</div>
        <div className="three-panel-table">{tablePanel}</div>
      </div>
    </div>
  );
}

// ---- Chat -----------------------------------------------------------------

function ChatDemoFull() {
  return (
    <ThreePanel
      code={
        <CodeStack
          files={[
            {
              name: "chat/Channel.tsx",
              langLabel: "TSX",
              lang: "ts",
              code: `import { db } from "@pylon/client";

export function Channel({ id }: { id: string }) {
  const messages = db.useQuery("Message", {
    where: { channelId: id },
    order: "desc",
    limit: 50,
  });

  const send = db.useMutation("send");

  return (
    <Pane>
      <List items={messages ?? []} />
      <Composer onSubmit={body =>
        send({ channelId: id, body })
      } />
    </Pane>
  );
}`,
            },
            {
              name: "app.ts",
              langLabel: "TS",
              lang: "ts",
              code: `import { entity, v } from "@pylon/server";

export const Message = entity({
  fields: {
    channelId: v.id("Channel"),
    authorId:  v.id("User"),
    body:      v.string().min(1).max(4000),
    createdAt: v.timestamp(),
  },
  indexes: [
    { on: ["channelId", "createdAt"] },
  ],
  policy: {
    read:  (ctx) => ctx.user != null,
    write: (ctx, row) => row.authorId === ctx.user.id,
  },
});`,
            },
          ]}
        />
      }
      appPanel={<ChatDemo />}
      tablePanel={
        <DataTable
          title="Message"
          tag="live · 3 rows"
          columns={["_id", "authorId", "body", "createdAt"]}
          rows={[
            ["msg_7a1b…", "usr_maya", "shipping the v0.8 tick loop tonight", "3s ago"],
            ["msg_7a1c…", "usr_jonas", "pulled it — tests pass on my laptop", "12s ago"],
            ["msg_7a1d…", "usr_rhea", "useQuery fires in 4ms here", "1m ago"],
          ]}
        />
      }
    />
  );
}

// ---- Dashboard ------------------------------------------------------------

function DashboardApp() {
  // Rotating "live" metrics — numbers tick up to feel alive.
  const [tick, setTick] = React.useState(0);
  React.useEffect(() => {
    const t = setInterval(() => setTick((x) => x + 1), 1600);
    return () => clearInterval(t);
  }, []);
  const revenue = 12439 + tick * 17;
  const orders = 142 + Math.floor(tick / 3);
  const online = 47 + (tick % 5);
  const bars = [32, 48, 61, 58, 72, 84, 76, 92, 68, 74, 88, 96]
    .map((v, i) => (i === 11 ? 40 + ((tick * 3) % 60) : v));

  return (
    <div className="dash-app">
      <div className="dash-head">
        <div className="dash-head-title">
          <span className="mono text-dim" style={{ fontSize: 11 }}>#</span>
          admin · overview
        </div>
        <span className="status-pill" style={{ padding: "3px 8px", fontSize: 10.5 }}>
          <span className="dot" /> live
        </span>
      </div>
      <div className="dash-metrics">
        <div className="dash-metric">
          <div className="dash-metric-label">Revenue (24h)</div>
          <div className="dash-metric-value">${revenue.toLocaleString("en-US")}</div>
          <div className="dash-metric-delta up">↑ 12.4%</div>
        </div>
        <div className="dash-metric">
          <div className="dash-metric-label">Orders</div>
          <div className="dash-metric-value">{orders}</div>
          <div className="dash-metric-delta up">↑ 3 new</div>
        </div>
        <div className="dash-metric">
          <div className="dash-metric-label">Online</div>
          <div className="dash-metric-value">{online}</div>
          <div className="dash-metric-delta">steady</div>
        </div>
      </div>
      <div className="dash-chart">
        {bars.map((h, i) => (
          <div
            key={i}
            className={`dash-bar ${i === 11 ? "live" : ""}`}
            style={{ height: `${h}%` }}
          />
        ))}
      </div>
      <div className="dash-chart-x">
        <span>00</span><span>04</span><span>08</span><span>12</span><span>16</span><span>20</span>
      </div>
    </div>
  );
}

function DashboardDemoFull() {
  return (
    <ThreePanel
      code={
        <CodeStack
          files={[
            {
              name: "admin/Dashboard.tsx",
              langLabel: "TSX",
              lang: "ts",
              code: `import { db } from "@pylon/client";

export function Dashboard() {
  const revenue = db.useAggregate("Order", {
    sum: "total",
    where: { createdAt: { gte: "-24h" } },
  });
  const orders = db.useQuery("Order", {
    order: "desc", limit: 12,
  });
  const online = db.usePresence("admin");

  return (
    <Grid>
      <Metric label="Revenue (24h)" value={revenue} />
      <Metric label="Orders" value={orders.length} />
      <Metric label="Online" value={online.count} />
      <Chart series={orders.hourlyTotals()} />
    </Grid>
  );
}`,
            },
            {
              name: "app.ts",
              langLabel: "TS",
              lang: "ts",
              code: `import { entity, v } from "@pylon/server";

export const Order = entity({
  fields: {
    customerId: v.id("Customer"),
    total:      v.money("USD"),
    status:     v.enum(["pending", "paid", "refunded"]),
    createdAt:  v.timestamp(),
  },
  indexes: [
    { on: ["createdAt"] },
    { on: ["status", "createdAt"] },
  ],
  aggregates: {
    total: v.sum("total"),
  },
});`,
            },
          ]}
        />
      }
      appPanel={<DashboardApp />}
      tablePanel={
        <DataTable
          title="Order"
          tag="live · 4 rows"
          columns={["_id", "customer", "total", "status", "createdAt"]}
          rows={[
            ["ord_9f2a…", "Jordan Moss", "$89.00", <span key="1" className="pill-ok">paid</span>, "3s ago"],
            ["ord_9f2b…", "Rhea Patel", "$145.00", <span key="2" className="pill-ok">paid</span>, "22s ago"],
            ["ord_9f2c…", "Maya Torres", "$22.50", <span key="3" className="pill-warn">pending</span>, "1m ago"],
            ["ord_9f2d…", "Alex Chen", "$312.00", <span key="4" className="pill-ok">paid</span>, "4m ago"],
          ]}
        />
      }
    />
  );
}

// ---- Shard ----------------------------------------------------------------

function ShardDemoFull() {
  return (
    <ThreePanel
      code={
        <CodeStack
          files={[
            {
              name: "shards/match.rs",
              langLabel: "RUST",
              lang: "rust",
              code: `#[shard(tps = 20)]
pub fn match_shard(state: &mut State, input: Input) {
    for entity in state.entities.iter_mut() {
        entity.apply(input.movement);
        entity.tick();
    }

    state.aoi(150).broadcast(|e| e.snapshot());
}

// client: subscribes, renders, sends inputs.
const { state, send } = useShard("match_1");`,
            },
            {
              name: "app.ts",
              langLabel: "TS",
              lang: "ts",
              code: `import { shard, v } from "@pylon/server";

export const Match = shard({
  fields: {
    matchId:  v.id("Match"),
    entities: v.list(v.ref("Entity")),
    tick:     v.u32(),
  },
  tps: 20,
  aoi: { radius_m: 150 },
});`,
            },
          ]}
        />
      }
      appPanel={
        <div className="shard-app">
          <GameCanvas />
        </div>
      }
      tablePanel={
        <DataTable
          title="Entity"
          tag="live · 4 rows"
          columns={["_id", "x", "y", "vel", "hp"]}
          rows={[
            ["ent_p1", "0.42", "0.38", "0.04", "98"],
            ["ent_npc_a", "0.71", "0.62", "0.02", "100"],
            ["ent_npc_b", "0.23", "0.84", "0.03", "100"],
            ["ent_npc_c", "0.58", "0.19", "0.05", "72"],
          ]}
        />
      }
    />
  );
}

// ---- Dev ------------------------------------------------------------------

function DevDemoFull() {
  return (
    <ThreePanel
      code={
        <CodeStack
          files={[
            {
              name: "app.ts",
              langLabel: "TS",
              lang: "ts",
              code: `import { entity, v } from "@pylon/server";

export const Message = entity({
  fields: {
    channelId: v.id("Channel"),
    authorId:  v.id("User"),
    body:      v.string().min(1).max(4000),
    createdAt: v.timestamp(),
  },
  indexes: [
    { on: ["channelId", "createdAt"] },
  ],
});`,
            },
            {
              name: "pylon.json",
              langLabel: "JSON",
              lang: "ts",
              code: `{
  "name": "my-app",
  "version": "0.1.0",
  "storage": "sqlite:.pylon/dev.db",
  "bindings": {
    "client": "web/src/pylon.client.ts"
  }
}`,
            },
          ]}
        />
      }
      appPanel={
        <div className="dev-app">
          <HeroTerminal />
        </div>
      }
      tablePanel={
        <DataTable
          title="Schema"
          tag="12 tables loaded"
          columns={["name", "fields", "indexes", "policies"]}
          rows={[
            ["Message", "5", "1", "read, write"],
            ["Channel", "4", "1", "read"],
            ["User", "6", "2", "read, write"],
            ["Session", "3", "1", "—"],
          ]}
        />
      }
    />
  );
}

export function Hero() {
  return (
    <section className="hero">
      <div className="hero-grid-bg" />
      <div className="container-page hero-inner">
        <div className="hero-eyebrow">
          <span className="chip">NEW</span>
          <span>Pylon Cloud — deploy anywhere, idle at $0</span>
          <span className="arrow">→</span>
        </div>

        <h1 className="hero-h1">
          The backend for
          <br />
          <em>real-time</em> apps and games.
        </h1>

        <p className="hero-sub">
          Declarative schema, live sync, TypeScript functions, and tick-based game
          shards — as a single Rust binary. Run it on a <code>VPS</code>,{" "}
          <code>AWS</code>, or <code>Cloudflare Workers</code>.
        </p>

        <div className="cta-row">
          <InstallCmd />
          <GithubButton />
        </div>

        <HeroDemo />
      </div>
    </section>
  );
}
