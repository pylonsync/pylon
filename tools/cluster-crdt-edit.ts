// CRDT edit helper for the cluster smoke: fetch the row's current
// Loro state via a fresh crdt-subscribe catch-up frame, apply a text
// edit to the SERVER-CONTRACT container (root "row" map → LoroText
// child — see crates/crdt ROOT_MAP), and POST the update back.
//
//   bun cluster-crdt-edit.ts <baseUrl> <token> <entity> <rowId> <text-to-append>
//
// Run from a directory whose node_modules carries loro-crdt
// (examples/pad does).

// Resolve loro-crdt from the CALLER's cwd (the app dir), not this
// script's directory — bun resolves bare imports from the importing
// file, and tools/ has no node_modules.
import { createRequire } from "node:module";
const requireFromApp = createRequire(`${process.cwd()}/`);
const { LoroDoc } = requireFromApp("loro-crdt");

const [baseUrl, token, entity, rowId, text] = process.argv.slice(2);
if (!text) {
  console.error("usage: cluster-crdt-edit.ts <baseUrl> <token> <entity> <rowId> <text>");
  process.exit(64);
}

// 1. Catch-up snapshot over WS (same frames the browser client gets).
const wsUrl = baseUrl.replace(/^http/, "ws") + "/api/sync/ws";
const snapshot: Uint8Array = await new Promise((resolve, reject) => {
  const ws = new WebSocket(wsUrl, [`bearer.${token}`]);
  ws.binaryType = "arraybuffer";
  const t = setTimeout(() => reject(new Error("no catch-up frame within 10s")), 10000);
  ws.addEventListener("open", () => {
    ws.send(JSON.stringify({ type: "crdt-subscribe", entity, rowId }));
  });
  ws.addEventListener("message", (ev) => {
    if (ev.data instanceof ArrayBuffer) {
      clearTimeout(t);
      ws.close();
      resolve(new Uint8Array(ev.data));
    }
  });
  ws.addEventListener("error", () => reject(new Error("ws error")));
});

// 2. Strip the routing header ([type u8][entity_len u16][entity][rowid_len u16][rowid])
//    — canonical layout in crates/router/src/lib.rs.
const view = new DataView(snapshot.buffer, snapshot.byteOffset, snapshot.byteLength);
const entityLen = view.getUint16(1, false);
const rowIdLen = view.getUint16(3 + entityLen, false);
const payload = snapshot.slice(3 + entityLen + 2 + rowIdLen);

// 3. Import → edit through the row map → export the delta.
const doc = new LoroDoc();
doc.import(payload);
const row = doc.getMap("row");
const content = row.get("content");
if (!content || (content as any).kind?.() !== "Text") {
  console.error("row map has no LoroText 'content' — wrong entity shape for this test");
  process.exit(1);
}
const textContainer = content as any;
textContainer.insert(textContainer.length, text);
doc.commit();
const update = doc.export({ mode: "update" });

// 4. POST the update to the machine under test. The route takes JSON
// with hex-encoded Loro bytes (crates/router/src/routes/crdt.rs).
const hex = Array.from(update as Uint8Array)
  .map((b) => b.toString(16).padStart(2, "0"))
  .join("");
const res = await fetch(`${baseUrl}/api/crdt/${entity}/${rowId}`, {
  method: "POST",
  headers: {
    authorization: `Bearer ${token}`,
    "content-type": "application/json",
  },
  body: JSON.stringify({ update: hex }),
});
console.log(`EDIT_POST ${res.status}`);
process.exit(res.ok ? 0 : 1);
