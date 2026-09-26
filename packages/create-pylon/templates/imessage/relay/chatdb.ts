// Read new messages from macOS Messages' database (~/Library/Messages/chat.db).
//
// The database is opened READ-ONLY. Messages.app owns it; this script never
// writes to it. New rows are found by ROWID watermark: every message row gets
// a larger ROWID than the ones before it.

import { Database } from "bun:sqlite";

/** Seconds between 1970-01-01 and 2001-01-01 (Apple's reference date). */
const APPLE_EPOCH_OFFSET_S = 978_307_200;

export interface ChatDbMessage {
  rowId: number;
  guid: string;
  handle: string;
  text: string;
  sentAt: string;
  service: string;
  /** Set when the row belongs to a group chat; the relay skips those. */
  isGroup: boolean;
}

export function openChatDb(path: string): Database {
  return new Database(path, { readonly: true });
}

/** message.date: nanoseconds since 2001 on macOS 10.13+, seconds before that. */
export function appleDateToIso(value: number | bigint | null): string {
  if (value === null || value === undefined) return new Date().toISOString();
  const n = Number(value);
  if (!Number.isFinite(n) || n === 0) return new Date().toISOString();
  const seconds = n > 1e12 ? n / 1e9 : n;
  return new Date((seconds + APPLE_EPOCH_OFFSET_S) * 1000).toISOString();
}

/**
 * Pull the plain text out of `message.attributedBody`, an NSAttributedString
 * archived with NSArchiver (typedstream). Since macOS 13 many rows have a
 * null `text` column and carry the text only here.
 *
 * Layout around the string: the class name "NSString", five bytes of
 * typedstream framing ending in 0x2B ('+'), then a length and the UTF-8
 * bytes. The length is one byte, or 0x81 followed by a little-endian uint16,
 * or 0x82 followed by a little-endian uint32.
 */
export function decodeAttributedBody(blob: Uint8Array | null | undefined): string | null {
  if (!blob || blob.length === 0) return null;
  const bytes = blob instanceof Uint8Array ? blob : new Uint8Array(blob);
  const marker = new TextEncoder().encode("NSString");
  const start = indexOf(bytes, marker);
  if (start === -1) return null;
  let i = start + marker.length;
  // Find the '+' that introduces the string payload within the framing bytes.
  const plus = bytes.indexOf(0x2b, i);
  if (plus === -1 || plus - i > 8) return null;
  i = plus + 1;
  if (i >= bytes.length) return null;
  let length = bytes[i];
  i += 1;
  if (length === 0x81) {
    if (i + 2 > bytes.length) return null;
    length = bytes[i] | (bytes[i + 1] << 8);
    i += 2;
  } else if (length === 0x82) {
    if (i + 4 > bytes.length) return null;
    length = (bytes[i] | (bytes[i + 1] << 8) | (bytes[i + 2] << 16) | (bytes[i + 3] << 24)) >>> 0;
    i += 4;
  }
  if (length === 0 || i + length > bytes.length) return null;
  const text = new TextDecoder("utf-8", { fatal: false }).decode(bytes.subarray(i, i + length));
  return text.replace(/￼/g, "").trim() || null;
}

function indexOf(haystack: Uint8Array, needle: Uint8Array): number {
  outer: for (let i = 0; i <= haystack.length - needle.length; i++) {
    for (let j = 0; j < needle.length; j++) {
      if (haystack[i + j] !== needle[j]) continue outer;
    }
    return i;
  }
  return -1;
}

/** The newest ROWID, used as the starting watermark so old history is never replayed. */
export function latestRowId(db: Database): number {
  const row = db.query("SELECT COALESCE(MAX(ROWID), 0) AS max FROM message").get() as { max: number };
  return Number(row.max) || 0;
}

interface RawRow {
  rowId: number;
  guid: string;
  text: string | null;
  attributedBody: Uint8Array | null;
  date: number | bigint | null;
  service: string | null;
  handle: string | null;
  chatStyle: number | null;
  participants: number | null;
}

/**
 * Inbound messages after `afterRowId`, oldest first. Skips the owner's own
 * sent messages, tapbacks and other reactions (associated_message_type != 0),
 * and rows with no sender handle. Group-chat rows are returned with
 * `isGroup: true` so the caller can advance past them.
 */
export function readNewMessages(db: Database, afterRowId: number, limit = 100): ChatDbMessage[] {
  const rows = db
    .query(
      `SELECT m.ROWID AS rowId, m.guid AS guid, m.text AS text, m.attributedBody AS attributedBody,
              m.date AS date, m.service AS service, h.id AS handle,
              c.style AS chatStyle,
              (SELECT COUNT(*) FROM chat_handle_join chj WHERE chj.chat_id = c.ROWID) AS participants
         FROM message m
         LEFT JOIN handle h ON h.ROWID = m.handle_id
         LEFT JOIN chat_message_join cmj ON cmj.message_id = m.ROWID
         LEFT JOIN chat c ON c.ROWID = cmj.chat_id
        WHERE m.ROWID > ?1
          AND m.is_from_me = 0
          AND COALESCE(m.associated_message_type, 0) = 0
        GROUP BY m.ROWID
        ORDER BY m.ROWID ASC
        LIMIT ?2`,
    )
    .all(afterRowId, limit) as RawRow[];

  const out: ChatDbMessage[] = [];
  for (const r of rows) {
    const text = r.text && r.text.trim() !== "" ? r.text : decodeAttributedBody(r.attributedBody);
    out.push({
      rowId: Number(r.rowId),
      guid: String(r.guid),
      handle: r.handle ?? "",
      text: (text ?? "").replace(/￼/g, "").trim(),
      sentAt: appleDateToIso(r.date),
      service: r.service ?? "iMessage",
      // chat.style 43 is a group chat, 45 a one-to-one chat.
      isGroup: r.chatStyle === 43 || Number(r.participants ?? 0) > 1,
    });
  }
  return out;
}

/**
 * The new watermark after a sync. Walks the batch in ROWID order and stops at
 * the first message that was sent to the server but not accepted, so that
 * message is read again next time. Rows the relay skipped (group chats,
 * reactions without text) are passed over.
 */
export function advanceWatermark(
  current: number,
  rows: { rowId: number; guid: string }[],
  sent: { guid: string }[],
  accepted: string[],
): number {
  const sentGuids = new Set(sent.map((m) => m.guid));
  const acceptedGuids = new Set(accepted);
  let watermark = current;
  for (const r of rows) {
    if (sentGuids.has(r.guid) && !acceptedGuids.has(r.guid)) break;
    if (r.rowId > watermark) watermark = r.rowId;
  }
  return watermark;
}
