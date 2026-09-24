"use client";

import React, { useEffect, useRef, useState } from "react";
import { callFn, configureClient, init, storageKey, useShard } from "@pylonsync/react";

const APP_NAME = "shard-arena";

interface Player {
  id: string;
  x: number;
  y: number;
  tx: number;
  ty: number;
  hue: number;
}

interface Arena {
  width: number;
  height: number;
  players: Player[];
}

type Input = "join" | { move_to: { x: number; y: number } };

interface Join {
  shardId: string;
  subscriberId: string;
  ticket: string;
}

/** Sign in as a guest (once per browser) and get a ticket for the arena. */
async function join(): Promise<Join> {
  init({ appName: APP_NAME });
  configureClient({ appName: APP_NAME });
  if (!window.localStorage.getItem(storageKey("token"))) {
    const res = await fetch("/api/auth/guest", { method: "POST" });
    if (!res.ok) throw new Error(`guest sign-in failed: ${res.status}`);
    const body = (await res.json()) as { token: string; user_id: string };
    window.localStorage.setItem(storageKey("token"), body.token);
    window.localStorage.setItem(storageKey("user"), body.user_id);
    configureClient({ appName: APP_NAME });
  }
  return callFn<Join>("joinArena", {});
}

export default function ArenaIsland() {
  const [session, setSession] = useState<Join | null>(null);
  const [failure, setFailure] = useState<string | null>(null);

  useEffect(() => {
    join().then(setSession, (e: unknown) => setFailure(String(e)));
  }, []);

  if (failure) return <p style={{ padding: 24 }}>Could not join: {failure}</p>;
  if (!session) return <p style={{ padding: 24 }}>Joining the arena…</p>;
  return <ArenaView session={session} />;
}

function ArenaView({ session }: { session: Join }) {
  const { snapshot, connected, send, lastRejection } = useShard<Arena, Input>(session.shardId, {
    subscriberId: session.subscriberId,
    ticket: session.ticket,
  });
  const canvas = useRef<HTMLCanvasElement | null>(null);

  // Say hello once connected, and every 20 s so the arena keeps this
  // player while the tab is open.
  useEffect(() => {
    if (!connected) return;
    send("join");
    const timer = setInterval(() => send("join"), 20_000);
    return () => clearInterval(timer);
  }, [connected, send]);

  useEffect(() => {
    const ctx = canvas.current?.getContext("2d");
    if (!ctx || !snapshot) return;
    ctx.clearRect(0, 0, snapshot.width, snapshot.height);
    for (const p of snapshot.players) {
      ctx.fillStyle = `hsl(${p.hue} 80% 60%)`;
      ctx.beginPath();
      ctx.arc(p.x, p.y, p.id === session.subscriberId ? 10 : 7, 0, Math.PI * 2);
      ctx.fill();
    }
  }, [snapshot, session.subscriberId]);

  const onClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    send({ move_to: { x: e.clientX - rect.left, y: e.clientY - rect.top } });
  };

  return (
    <main style={{ padding: 24 }}>
      <h1 style={{ fontSize: 20, margin: "0 0 12px" }}>Shard Arena</h1>
      <p style={{ margin: "0 0 12px", opacity: 0.7 }}>
        {connected ? `${snapshot?.players.length ?? 0} players` : "Connecting…"} · click to move
        {lastRejection ? ` · refused: ${lastRejection.message}` : ""}
      </p>
      <canvas
        ref={canvas}
        width={snapshot?.width ?? 800}
        height={snapshot?.height ?? 500}
        onClick={onClick}
        style={{ background: "#1b1b22", borderRadius: 8, cursor: "crosshair", maxWidth: "100%" }}
      />
    </main>
  );
}
