// The relay's small on-disk state: the chat.db ROWID watermark and the ids of
// replies it already sent but has not had acknowledged yet. Written with a
// temp file + rename, so a crash mid-write leaves the previous state intact.

import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

export interface RelayState {
  /** Highest chat.db ROWID the server has accepted. */
  lastRowId: number;
  /** Replies sent through Messages.app whose ack the server has not received. */
  sentUnacked: string[];
}

export function loadState(path: string): RelayState | null {
  if (!existsSync(path)) return null;
  try {
    const raw = JSON.parse(readFileSync(path, "utf8")) as Partial<RelayState>;
    const lastRowId = Number(raw.lastRowId);
    if (!Number.isFinite(lastRowId) || lastRowId < 0) return null;
    return {
      lastRowId,
      sentUnacked: Array.isArray(raw.sentUnacked) ? raw.sentUnacked.filter((s) => typeof s === "string") : [],
    };
  } catch {
    return null;
  }
}

export function saveState(path: string, state: RelayState): void {
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  const tmp = `${path}.${process.pid}.tmp`;
  writeFileSync(tmp, JSON.stringify(state), { mode: 0o600 });
  renameSync(tmp, path);
}
