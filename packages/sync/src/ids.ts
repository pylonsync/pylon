// ---------------------------------------------------------------------------
// Pylon-compatible id generation.
//
// IDs are lex-sortable: 32-char hex timestamp (BigInt nanoseconds)
// followed by 8-char hex counter. That contract is what the cursor
// pagination relies on — `WHERE id > '<after>'` returns rows in the
// same order across pages, and a 39-char id never sorts ahead of a
// 40-char id (which would corrupt pagination at the width boundary).
// ---------------------------------------------------------------------------

import type { Storage } from "./storage";

let idCounter = 0;

/**
 * Mint a Pylon-shaped id (40 hex chars). Used by `SyncEngine.insert`
 * so the optimistic ghost and the canonical row share the exact same
 * id — without this, the server-minted id would replace a `_pending_*`
 * placeholder and the local replica would briefly carry both.
 */
export function generateId(): string {
  // BigInt to dodge the 2^53 ceiling — `Date.now() * 1_000_000` busts
  // Number.MAX_SAFE_INTEGER for any timestamp past 1973. Hex output is
  // padded to 32 chars so the lex sort behaves at width boundaries.
  const nanos = BigInt(Date.now()) * 1_000_000n;
  const seq = idCounter++ >>> 0;
  return (
    nanos.toString(16).padStart(32, "0") + seq.toString(16).padStart(8, "0")
  );
}

/**
 * Stable client id persisted in storage. Lets the server correlate
 * retries (even when op_id is absent) and attach per-client
 * diagnostics / rate limits.
 */
export function generateClientId(storage: Storage): string {
  const key = "pylon:client_id";
  const existing = storage.get(key);
  if (existing) return existing;
  const fresh = newUuidLike();
  storage.set(key, fresh);
  return fresh;
}

function newUuidLike(): string {
  try {
    if (
      typeof crypto !== "undefined" &&
      typeof crypto.randomUUID === "function"
    ) {
      return crypto.randomUUID();
    }
  } catch {
    /* fall through */
  }
  const rand = Math.random().toString(36).slice(2, 10);
  const t = Date.now().toString(36);
  return `cl_${t}_${rand}`;
}
