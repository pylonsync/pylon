// WS probe for the two-machine cluster smoke (tools/smoke-cluster.sh).
//
// Speaks the raw Pylon sync WebSocket protocol (auth via the
// `bearer.<token>` subprotocol, JSON control frames, length-prefixed
// binary CRDT frames) so the test exercises the real wire — no client
// SDK in the loop to mask a server-side fanout gap.
//
// Modes (all wait up to --timeout ms, print observed events as lines,
// exit 0 iff --expect events were observed):
//
//   bun cluster-ws-probe.ts crdt   <wsUrl> <token> <entity> <rowId> --expect N
//   bun cluster-ws-probe.ts room   <wsUrl> <token> <room>           --expect N [--match S]
//   bun cluster-ws-probe.ts change <wsUrl> <token> <entity>         --expect N
//
// Output protocol: one line per observed event ("CRDT_FRAME <bytes>",
// "ROOM <json>", "CHANGE <json>"), then "PROBE_OK <count>" or
// "PROBE_TIMEOUT <count>/<expected>".

const [mode, wsUrl, token, ...rest] = process.argv.slice(2);

function flag(name: string, dflt: string): string {
  const i = rest.indexOf(name);
  return i >= 0 && rest[i + 1] ? rest[i + 1] : dflt;
}
const expect = Number(flag("--expect", "1"));
const timeoutMs = Number(flag("--timeout", "15000"));
const match = flag("--match", "");

if (!mode || !wsUrl || !token) {
  console.error(
    "usage: cluster-ws-probe.ts <crdt|room|change> <wsUrl> <token> [args] --expect N",
  );
  process.exit(64);
}

const positional = rest.filter((a, i) => !a.startsWith("--") && (i === 0 || !rest[i - 1].startsWith("--")));

let seen = 0;
const ws = new WebSocket(wsUrl, [`bearer.${token}`]);
ws.binaryType = "arraybuffer";

const done = (ok: boolean) => {
  console.log(ok ? `PROBE_OK ${seen}` : `PROBE_TIMEOUT ${seen}/${expect}`);
  try {
    ws.close();
  } catch {}
  process.exit(ok ? 0 : 1);
};
const timer = setTimeout(() => done(seen >= expect), timeoutMs);

ws.addEventListener("open", () => {
  if (mode === "crdt") {
    const [entity, rowId] = positional;
    ws.send(JSON.stringify({ type: "crdt-subscribe", entity, rowId }));
  } else if (mode === "room") {
    const [room] = positional;
    ws.send(JSON.stringify({ type: "room-subscribe", room }));
  }
  // "change" mode: change events fan to every authed socket the row
  // policy allows — no subscribe control frame needed.
  console.error(`[probe] connected: ${mode}`);
});

ws.addEventListener("message", (ev) => {
  if (ev.data instanceof ArrayBuffer) {
    if (mode === "crdt") {
      console.log(`CRDT_FRAME ${ev.data.byteLength}`);
      seen++;
    }
  } else {
    const text = String(ev.data);
    if (mode === "room" && text.includes("room")) {
      if (!match || text.includes(match)) {
        console.log(`ROOM ${text.slice(0, 300)}`);
        seen++;
      }
    } else if (mode === "change") {
      const [entity] = positional;
      if (text.includes(entity) && (!match || text.includes(match))) {
        console.log(`CHANGE ${text.slice(0, 300)}`);
        seen++;
      }
    }
  }
  if (seen >= expect) {
    clearTimeout(timer);
    done(true);
  }
});

ws.addEventListener("error", () => {
  console.error("[probe] socket error");
});
ws.addEventListener("close", (ev) => {
  console.error(`[probe] closed (${ev.code})`);
  if (seen < expect) done(false);
});
