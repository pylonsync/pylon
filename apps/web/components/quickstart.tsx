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
    title: "Install",
    desc: "Grab the CLI with cargo. One binary — no services, no Docker required.",
    filename: "shell",
    lang: "sh",
    code: `❯ cargo install statecraft-cli
   Compiling statecraft-cli v0.8.2
    Finished release in 41.2s
❯ statecraft --version
statecraft 0.8.2`,
  },
  {
    num: "02",
    title: "Init a project",
    desc: "Scaffolds schema, server functions, and a typed client.",
    filename: "shell",
    lang: "sh",
    code: `❯ statecraft init my-app
  ✓ schema.json
  ✓ functions/
  ✓ shards/
  ✓ client/ (typed)
❯ cd my-app`,
  },
  {
    num: "03",
    title: "Run dev",
    desc: "Starts the server, watches your code, regenerates the client on every save.",
    filename: "shell",
    lang: "sh",
    code: `❯ statecraft dev
  schema  · 3 tables
  shards  · 0 rooms
  serving on http://localhost:4242
  ✓ hot-reload · client regenerated (42ms)`,
  },
  {
    num: "04",
    title: "Open the browser",
    desc: "Mount the client anywhere you already have React. Queries are reactive by default.",
    filename: "App.tsx",
    lang: "tsx",
    code: `import { AgentDBProvider, db } from "@/statecraft/client";

export default function App() {
  const tasks = db.useQuery("Task", { order: "desc" });
  return (
    <AgentDBProvider url="http://localhost:4242">
      <List items={tasks ?? []} />
    </AgentDBProvider>
  );
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
          No account, no API keys, no waitlist. If you have cargo and a browser,
          you&apos;re set.
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
