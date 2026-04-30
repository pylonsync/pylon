"use client";

import * as React from "react";
import { highlightLine, type Lang } from "@/lib/highlight";

const QS_STEPS: {
  num: string;
  title: string;
  desc: string;
  filename: string;
  lang: Lang;
  code: string;
}[] = [
  {
    num: "01",
    title: "Scaffold",
    desc: "One npm command. Generates a Pylon backend + Next.js frontend in a single workspace — no global binary, no Rust toolchain, no Docker required.",
    filename: "shell",
    lang: "sh",
    code: `❯ npm create pylon@latest my-app
  Creating my-app in ./my-app
  ✓ Scaffolded api/ + web/ + shared schema
❯ cd my-app`,
  },
  {
    num: "02",
    title: "Install",
    desc: "Pulls @pylonsync/cli (platform-specific binary) + @pylonsync/sdk + @pylonsync/react. No global install.",
    filename: "shell",
    lang: "sh",
    code: `❯ npm install
  added 421 packages in 6s
  → @pylonsync/cli installed (darwin-arm64)
  → @pylonsync/sdk, @pylonsync/react ready`,
  },
  {
    num: "03",
    title: "Run dev",
    desc: "Spins up the API + web together. Watches your schema, regenerates the typed client on every save.",
    filename: "shell",
    lang: "sh",
    code: `❯ npm run dev
  api  http://localhost:4321
  web  http://localhost:3000
  ✓ schema synced — 1 entity, 1 query, 1 action`,
  },
  {
    num: "04",
    title: "Connect from React",
    desc: "One init call, then useQuery subscribes and restreams on every change.",
    filename: "App.tsx",
    lang: "tsx",
    code: `import { init, db } from "@pylonsync/react";

init({ baseUrl: "http://localhost:4321", appName: "my-app" });

export default function App() {
  const { data: tasks } = db.useQuery("Task");
  return <List items={tasks ?? []} />;
}`,
  },
];

export function Quickstart() {
  const [active, setActive] = React.useState(0);
  React.useEffect(() => {
    const t = setInterval(
      () => setActive((a) => (a + 1) % QS_STEPS.length),
      4500,
    );
    return () => clearInterval(t);
  }, []);
  const cur = QS_STEPS[active];
  return (
    <section className="section" id="quickstart">
      <div className="container-page">
        <div className="section-label">Quickstart</div>
        <h2 className="section-title">Four commands to a running backend.</h2>
        <p className="section-sub">
          No account, no API keys, no waitlist. One curl, one binary, one
          dev server.
        </p>

        <div className="quickstart">
          <div className="qs-steps">
            {QS_STEPS.map((s, i) => (
              <div
                key={i}
                className={`qs-step ${i === active ? "active" : ""}`}
                onClick={() => setActive(i)}
              >
                <div className="qs-step-num">{s.num}</div>
                <div className="qs-step-body">
                  <h4>{s.title}</h4>
                  <p>{s.desc}</p>
                </div>
              </div>
            ))}
          </div>
          <div className="qs-preview">
            <div className="panel">
              <div className="codeblock-header">
                <span className="filename">{cur.filename}</span>
                <span className="lang">{cur.lang}</span>
              </div>
              <pre className="code" style={{ margin: 0, minHeight: 220 }}>
                {cur.code.split("\n").map((ln, i) => {
                  const nodes = highlightLine(ln, cur.lang);
                  return (
                    <div key={`${active}-${i}`}>
                      {nodes.length ? nodes : "\u00A0"}
                    </div>
                  );
                })}
              </pre>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
