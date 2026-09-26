// iMessage relay for a Mac that stays on. Run with `bun relay/main.ts` (or
// `bun run relay` from the project root). See the README's "Mac relay" section
// for the Full Disk Access and Automation permissions it needs.
//
// Every POLL_MS it:
//   1. reads chat.db rows newer than its watermark (read-only),
//   2. sends them to the server along with acks for replies it sent,
//   3. advances the watermark past the rows the server accepted,
//   4. sends the replies the server handed back, through Messages.app.
//
// Configuration (environment, or relay/.env which Bun loads automatically):
//   PYLON_URL       your app's origin, e.g. https://assistant.example.com
//   RELAY_TOKEN     same value as RELAY_TOKEN on the server
//   CHAT_DB         default ~/Library/Messages/chat.db
//   RELAY_STATE     default ~/.pylon-imessage-relay/state.json
//   POLL_MS         default 3000, minimum 2500 (the server allows 30 syncs a minute)
//   RELAY_DRY_RUN   1 to log replies instead of sending them

import { hostname, homedir } from "node:os";
import { join } from "node:path";
import { normalizeHandle } from "../lib/handles";
import { MAX_INBOUND_PER_SYNC, type RelayAck, type RelayInboundItem } from "../lib/relay-protocol";
import { sendIMessage } from "./applescript";
import { advanceWatermark, latestRowId, openChatDb, readNewMessages } from "./chatdb";
import { RelayHttpError, relaySync } from "./client";
import { loadState, saveState, type RelayState } from "./state";

const VERSION = "1.0.0";

function config() {
  const serverUrl = (process.env.PYLON_URL ?? "").trim();
  const token = (process.env.RELAY_TOKEN ?? "").trim();
  if (!/^https?:\/\//.test(serverUrl)) throw new Error("Set PYLON_URL to your app's origin, e.g. https://assistant.example.com");
  if (serverUrl.startsWith("http://") && !/^http:\/\/(localhost|127\.0\.0\.1)(:|\/|$)/.test(serverUrl)) {
    throw new Error("PYLON_URL must use https:// (http:// is allowed only for localhost)");
  }
  if (token.length < 24) throw new Error("Set RELAY_TOKEN (at least 24 characters, same value as the server)");
  return {
    serverUrl,
    token,
    chatDb: process.env.CHAT_DB || join(homedir(), "Library", "Messages", "chat.db"),
    statePath: process.env.RELAY_STATE || join(homedir(), ".pylon-imessage-relay", "state.json"),
    pollMs: Math.max(2500, Number(process.env.POLL_MS) || 3000),
    dryRun: ["1", "true", "yes"].includes((process.env.RELAY_DRY_RUN ?? "").toLowerCase()),
  };
}

const log = (msg: string) => console.log(`[relay ${new Date().toISOString()}] ${msg}`);

async function main() {
  const cfg = config();
  let db;
  try {
    db = openChatDb(cfg.chatDb);
  } catch (err) {
    throw new Error(
      `Cannot open ${cfg.chatDb}. Grant Full Disk Access to the app running this script (see README). ${err instanceof Error ? err.message : err}`,
    );
  }

  let state: RelayState = loadState(cfg.statePath) ?? { lastRowId: latestRowId(db), sentUnacked: [] };
  saveState(cfg.statePath, state);
  log(`started v${VERSION}; watermark ROWID ${state.lastRowId}${cfg.dryRun ? "; dry run, replies are not sent" : ""}`);

  // Acks for replies sent in a previous loop (or before a restart).
  let acks: RelayAck[] = state.sentUnacked.map((id) => ({ id, ok: true, dryRun: cfg.dryRun }));
  let stopping = false;
  process.on("SIGINT", () => (stopping = true));
  process.on("SIGTERM", () => (stopping = true));

  while (!stopping) {
    try {
      const rows = readNewMessages(db, state.lastRowId, MAX_INBOUND_PER_SYNC);
      const inbound: RelayInboundItem[] = [];
      for (const r of rows) {
        const handle = normalizeHandle(r.handle)?.handle;
        if (r.isGroup || !handle || r.text === "") continue;
        inbound.push({ guid: r.guid, handle, text: r.text, sentAt: r.sentAt, service: r.service });
      }

      const res = await relaySync(cfg.serverUrl, cfg.token, {
        relayVersion: VERSION,
        host: hostname().slice(0, 64),
        inbound,
        acks,
      });

      // The server stored the acks; forget them.
      const ackedIds = new Set(acks.map((a) => a.id));
      state = { ...state, sentUnacked: state.sentUnacked.filter((id) => !ackedIds.has(id)) };
      acks = [];

      state = { ...state, lastRowId: advanceWatermark(state.lastRowId, rows, inbound, res.accepted) };
      saveState(cfg.statePath, state);
      if (inbound.length > 0) log(`delivered ${inbound.length} inbound message(s)`);

      for (const item of res.outbound) {
        if (state.sentUnacked.includes(item.id)) {
          acks.push({ id: item.id, ok: true, dryRun: cfg.dryRun });
          continue;
        }
        const handle = normalizeHandle(item.handle)?.handle;
        const result = handle
          ? await sendIMessage(handle, item.text, { dryRun: cfg.dryRun })
          : ({ ok: false, error: "invalid recipient" } as const);
        if (result.ok) {
          // Persist before acking so a crash here never sends twice.
          state = { ...state, sentUnacked: [...state.sentUnacked, item.id] };
          saveState(cfg.statePath, state);
          acks.push({ id: item.id, ok: true, dryRun: cfg.dryRun });
        } else {
          acks.push({ id: item.id, ok: false, error: result.error });
          log(`send failed: ${result.error}`);
        }
      }
      if (res.outbound.length > 0) log(`${cfg.dryRun ? "dry run: skipped" : "sent"} ${res.outbound.length} repl${res.outbound.length === 1 ? "y" : "ies"}`);
    } catch (err) {
      if (err instanceof RelayHttpError && (err.code === "UNAUTHORIZED" || err.code === "NOT_CONFIGURED")) {
        log(`server rejected the relay (${err.code}). Check RELAY_TOKEN on both sides.`);
      } else if (err instanceof RelayHttpError && err.code === "TRANSPORT_DISABLED") {
        log("server is not using the relay transport. Set IMESSAGE_TRANSPORT=relay on the server.");
      } else {
        log(`sync failed: ${err instanceof Error ? err.message : String(err)}`);
      }
    }
    await Bun.sleep(cfg.pollMs);
  }
  db.close();
  log("stopped");
}

if (import.meta.main) {
  main().catch((err) => {
    console.error(`[relay] ${err instanceof Error ? err.message : String(err)}`);
    process.exit(1);
  });
}
