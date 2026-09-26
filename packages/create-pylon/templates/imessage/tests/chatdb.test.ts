import { Database } from "bun:sqlite";
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  advanceWatermark,
  appleDateToIso,
  decodeAttributedBody,
  latestRowId,
  openChatDb,
  readNewMessages,
} from "../relay/chatdb";

// A fixture with the columns of macOS Messages' chat.db that the relay reads.
// Built from scratch in a temp dir; the real ~/Library/Messages/chat.db is
// never touched.
const SCHEMA = `
CREATE TABLE handle (ROWID INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL, service TEXT);
CREATE TABLE chat (ROWID INTEGER PRIMARY KEY AUTOINCREMENT, guid TEXT, style INTEGER, chat_identifier TEXT);
CREATE TABLE chat_handle_join (chat_id INTEGER, handle_id INTEGER);
CREATE TABLE message (
  ROWID INTEGER PRIMARY KEY AUTOINCREMENT,
  guid TEXT UNIQUE NOT NULL,
  text TEXT,
  attributedBody BLOB,
  handle_id INTEGER DEFAULT 0,
  service TEXT,
  date INTEGER,
  is_from_me INTEGER DEFAULT 0,
  associated_message_type INTEGER DEFAULT 0,
  cache_has_attachments INTEGER DEFAULT 0
);
CREATE TABLE chat_message_join (chat_id INTEGER, message_id INTEGER, message_date INTEGER);
`;

/** An NSArchiver typedstream blob with the shape Messages writes. */
function attributedBody(text: string): Uint8Array {
  const payload = new TextEncoder().encode(text);
  const head = new TextEncoder().encode("\u0004\u000bstreamtyped\u0081è\u0003\u0084\u0001@\u0084\u0084\u0084\u0012NSAttributedString\u0000\u0084\u0084\u0008NSObject\u0000\u0085\u0092\u0084\u0084\u0084\u0008NSString");
  const framing = [0x01, 0x94, 0x84, 0x01, 0x2b];
  const len =
    payload.length < 0x80
      ? [payload.length]
      : [0x81, payload.length & 0xff, (payload.length >> 8) & 0xff];
  const tail = new TextEncoder().encode("\u0086\u0084\u0002iI\u0001");
  return new Uint8Array([...head, ...framing, ...len, ...payload, ...tail]);
}

// 2026-09-26T15:00:00Z in Apple nanoseconds.
const APPLE_NS = BigInt(Date.parse("2026-09-26T15:00:00Z") / 1000 - 978_307_200) * 1_000_000_000n;

let path: string;

beforeAll(() => {
  path = join(mkdtempSync(join(tmpdir(), "chatdb-fixture-")), "chat.db");
  const db = new Database(path, { create: true });
  db.exec(SCHEMA);
  db.run("INSERT INTO handle (id, service) VALUES ('+15125550148', 'iMessage'), ('maya@example.com', 'iMessage'), ('+15125550163', 'iMessage')");
  db.run("INSERT INTO chat (guid, style, chat_identifier) VALUES ('iMessage;-;+15125550148', 45, '+15125550148'), ('iMessage;+;chat1', 43, 'chat1'), ('iMessage;-;maya@example.com', 45, 'maya@example.com')");
  db.run("INSERT INTO chat_handle_join VALUES (1, 1), (2, 1), (2, 3), (3, 2)");
  const insert = db.prepare(
    "INSERT INTO message (guid, text, attributedBody, handle_id, service, date, is_from_me, associated_message_type) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
  );
  const join_ = db.prepare("INSERT INTO chat_message_join VALUES (?, ?, 0)");
  const rows: [string, string | null, Uint8Array | null, number, number, number, number][] = [
    ["G-OLD", "old message", null, 1, 0, 0, 1], // 1: history before the watermark
    ["G-TEXT", "plain text body", null, 1, 0, 0, 1], // 2
    ["G-ATTR", null, attributedBody("only in attributedBody ✓"), 2, 0, 0, 3], // 3
    ["G-MINE", "sent by the owner", null, 1, 1, 0, 1], // 4: is_from_me
    ["G-TAPBACK", "Loved “plain text body”", null, 1, 0, 2000, 1], // 5: reaction
    ["G-GROUP", "hi all", null, 1, 0, 0, 2], // 6: group chat
    ["G-LONG", null, attributedBody("x".repeat(300)), 2, 0, 0, 3], // 7: 0x81 length
  ];
  for (const [guid, text, body, handle, fromMe, assoc, chat] of rows) {
    const r = insert.run(guid, text, body, handle, "iMessage", APPLE_NS, fromMe, assoc);
    join_.run(chat, Number(r.lastInsertRowid));
  }
  db.close();
});

afterAll(() => {});

describe("chat.db reader", () => {
  test("opens read-only", () => {
    const db = openChatDb(path);
    expect(() => db.run("INSERT INTO handle (id) VALUES ('x')")).toThrow();
    db.close();
  });

  test("latestRowId is the newest message", () => {
    const db = openChatDb(path);
    expect(latestRowId(db)).toBe(7);
    db.close();
  });

  test("reads inbound rows after the watermark and skips own messages and reactions", () => {
    const db = openChatDb(path);
    const rows = readNewMessages(db, 1);
    db.close();
    expect(rows.map((r) => r.guid)).toEqual(["G-TEXT", "G-ATTR", "G-GROUP", "G-LONG"]);
    const [plain, attr, group, long] = rows;
    expect(plain).toMatchObject({ handle: "+15125550148", text: "plain text body", isGroup: false });
    expect(plain.sentAt).toBe("2026-09-26T15:00:00.000Z");
    expect(attr).toMatchObject({ handle: "maya@example.com", text: "only in attributedBody ✓", isGroup: false });
    expect(group.isGroup).toBe(true);
    expect(long.text).toBe("x".repeat(300));
  });

  test("the watermark advances past skipped rows but stops at an unaccepted message", () => {
    const rows = [
      { rowId: 2, guid: "G-TEXT" },
      { rowId: 3, guid: "G-ATTR" },
      { rowId: 6, guid: "G-GROUP" },
      { rowId: 7, guid: "G-LONG" },
    ];
    const sent = [{ guid: "G-TEXT" }, { guid: "G-ATTR" }, { guid: "G-LONG" }];
    expect(advanceWatermark(1, rows, sent, ["G-TEXT", "G-ATTR", "G-LONG"])).toBe(7);
    expect(advanceWatermark(1, rows, sent, ["G-TEXT"])).toBe(2);
    expect(advanceWatermark(1, rows, sent, [])).toBe(1);
    expect(advanceWatermark(1, rows.slice(2, 3), [], [])).toBe(6);
  });
});

describe("attributedBody and dates", () => {
  test("decodes short and 0x81-length strings; rejects junk", () => {
    expect(decodeAttributedBody(attributedBody("hi"))).toBe("hi");
    expect(decodeAttributedBody(attributedBody("é".repeat(100)))).toBe("é".repeat(100));
    expect(decodeAttributedBody(new Uint8Array([1, 2, 3]))).toBeNull();
    expect(decodeAttributedBody(null)).toBeNull();
    const truncated = attributedBody("hello world").slice(0, -12);
    expect(decodeAttributedBody(truncated)).toBeNull();
  });

  test("Apple dates in seconds and nanoseconds", () => {
    expect(appleDateToIso(APPLE_NS)).toBe("2026-09-26T15:00:00.000Z");
    expect(appleDateToIso(Number(APPLE_NS / 1_000_000_000n))).toBe("2026-09-26T15:00:00.000Z");
  });
});
