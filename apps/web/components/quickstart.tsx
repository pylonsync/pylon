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
    desc: "Build the CLI from source. One Rust binary — no services, no Docker required.",
    filename: "shell",
    lang: "sh",
    code: `❯ git clone https://github.com/pylonsync/pylon
❯ cd pylon && cargo install --path crates/cli --locked
❯ pylon --version
pylon 0.1.0`,
  },
  {
    num: "02",
    title: "Init a project",
    desc: "Scaffolds a TypeScript schema entry point you extend by hand.",
    filename: "shell",
    lang: "sh",
    code: `❯ pylon init my-app
  ✓ app.ts
  ✓ tsconfig.json
❯ cd my-app && bun add @pylonsync/sdk @pylonsync/functions`,
  },
  {
    num: "03",
    title: "Run dev",
    desc: "Starts the server, watches your code, regenerates the typed client on every save.",
    filename: "shell",
    lang: "sh",
    code: `❯ pylon dev app.ts
  ✓ my-app v0.1.0 — 1 entities, 0 queries, 0 actions, 1 policies
  Server:   http://localhost:4321
  Database: .pylon/dev.db (schema synced)`,
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
