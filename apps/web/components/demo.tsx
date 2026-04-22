"use client";

import * as React from "react";
import { CodeLines, type Lang } from "@/lib/highlight";

const APP_SCHEMA = `{
  "Message": {
    "fields": {
      "channelId": "id<Channel>",
      "authorId":  "id<User>",
      "body":      "string",
      "createdAt": "timestamp"
    },
    "indexes": [
      { "on": ["channelId", "createdAt"] }
    ]
  }
}`;

const APP_MUTATION = `import { mutation } from "statecraft/server";
import { z } from "zod";

export const send = mutation({
  args: {
    channelId: z.id("Channel"),
    body:      z.string().min(1).max(4000),
  },
  handler: async (ctx, { channelId, body }) => {
    const me = await ctx.auth.require();
    return ctx.db.insert("Message", {
      channelId,
      authorId:  me.id,
      body,
      createdAt: Date.now(),
    });
  },
});`;

const APP_HOOK = `import { db } from "@/statecraft/client";

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
      <Composer onSubmit={(body) =>
        send({ channelId: id, body })
      } />
    </Pane>
  );
}`;

const GAME_SIM = `use statecraft::shard::{SimState, Ctx, Input};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct Match {
    pub entities: Vec<Entity>,
    pub tick: u32,
}

impl SimState for Match {
    type Input = PlayerInput;

    fn tick(&mut self, ctx: &mut Ctx, inputs: &[Input<PlayerInput>]) {
        for input in inputs {
            if let Some(e) = self.entity_mut(input.player) {
                e.apply(input.payload);
            }
        }
        for e in &mut self.entities {
            e.integrate(ctx.dt);
        }
        self.tick += 1;
    }
}`;

const GAME_HOOK = `import { useShard } from "statecraft/react";

export function Match({ id }: { id: string }) {
  const { state, send, ping } = useShard("Match", id, {
    tps: 20,
    areaOfInterest: 64,
  });

  useKey("ArrowUp",    () => send({ move: "up" }));
  useKey("ArrowDown",  () => send({ move: "down" }));

  return (
    <Canvas tick={state.tick} entities={state.entities}>
      <Hud ping={ping} tps={20} />
    </Canvas>
  );
}`;

const GAME_SERVER = `// shards/match.ts — matchmaker + spawn
import { shard } from "statecraft/server";
import { Match } from "./sim";

export const match = shard("Match", {
  sim: Match,
  tps: 20,
  capacity: 64,
  matchmaker: "fill-then-new",
  onJoin: async (ctx, player) => {
    ctx.state.entities.push({
      id: player.id,
      x: 0, y: 0,
      color: ctx.palette.next(),
    });
  },
});`;

type Pane = { filename: string; lang: Lang; code: string; label: string };

export function DemoSection() {
  const [tab, setTab] = React.useState<"app" | "game">("app");
  const panes: Pane[] =
    tab === "app"
      ? [
          { filename: "schema.json", lang: "json", code: APP_SCHEMA, label: "1. Declare" },
          { filename: "functions/chat.ts", lang: "ts", code: APP_MUTATION, label: "2. Write" },
          { filename: "Channel.tsx", lang: "tsx", code: APP_HOOK, label: "3. Subscribe" },
        ]
      : [
          { filename: "shards/match.rs", lang: "rust", code: GAME_SIM, label: "1. Simulate" },
          { filename: "shards/match.ts", lang: "ts", code: GAME_SERVER, label: "2. Host" },
          { filename: "Match.tsx", lang: "tsx", code: GAME_HOOK, label: "3. Play" },
        ];

  return (
    <section className="section" id="demo">
      <div className="container-page">
        <div className="section-label">The 30-second demo</div>
        <h2 className="section-title">
          Schema. Function. Subscribe.
          <br />
          Or: simulate, host, play.
        </h2>
        <p className="section-sub">
          No migrations, no glue code, no separate realtime layer. The same server
          runs your app queries and your game ticks.
        </p>

        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            flexWrap: "wrap",
            gap: 16,
          }}
        >
          <div className="tabs">
            <button
              className={`tab ${tab === "app" ? "active" : ""}`}
              onClick={() => setTab("app")}
            >
              App<span className="kbd">1</span>
            </button>
            <button
              className={`tab ${tab === "game" ? "active" : ""}`}
              onClick={() => setTab("game")}
            >
              Game<span className="kbd">2</span>
            </button>
          </div>
          <div className="text-mono text-dim" style={{ fontSize: 12 }}>
            {tab === "app"
              ? "// collaborative chat — ~60 LOC server, ~20 LOC client"
              : "// authoritative multiplayer match — ~80 LOC server, ~25 LOC client"}
          </div>
        </div>

        <div className="demo-grid">
          {panes.map((p, i) => (
            <div key={`${tab}-${i}`} className="panel demo-card">
              <div className="codeblock-header">
                <span className="filename">
                  <span
                    className="text-accent text-mono"
                    style={{ marginRight: 8 }}
                  >
                    {p.label}
                  </span>
                  {p.filename}
                </span>
                <span className="lang">{p.lang}</span>
              </div>
              <pre className="code" style={{ margin: 0 }}>
                <CodeLines code={p.code} lang={p.lang} />
              </pre>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
