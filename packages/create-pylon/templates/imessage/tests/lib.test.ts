import { describe, expect, test } from "bun:test";
import { buildTurnInput, formatLocalTime } from "../lib/assistant";
import { formatHandle, normalizeHandle } from "../lib/handles";
import { parseRelayInbound } from "../lib/relay-protocol";
import { checkReminderTime, cleanStoredText } from "../lib/reminders";
import { selectedTransport, transportChecklist } from "../lib/transport";

describe("handles", () => {
  test("normalizes phone numbers and emails", () => {
    expect(normalizeHandle("(512) 555-0148")).toEqual({ handle: "+15125550148", kind: "phone" });
    expect(normalizeHandle("1-512-555-0148")).toEqual({ handle: "+15125550148", kind: "phone" });
    expect(normalizeHandle("+44 20 7946 0958")).toEqual({ handle: "+442079460958", kind: "phone" });
    expect(normalizeHandle("tel:+15125550148")).toEqual({ handle: "+15125550148", kind: "phone" });
    expect(normalizeHandle("iMessage;-;+15125550148")).toEqual({ handle: "+15125550148", kind: "phone" });
    expect(normalizeHandle(" Maya@Example.COM ")).toEqual({ handle: "maya@example.com", kind: "email" });
  });

  test("rejects anything else", () => {
    for (const bad of ["", "chat123456789", "12345", "+0123456789", "hello", "a@b", "5125550148; rm -rf", null, undefined]) {
      expect(normalizeHandle(bad as string)).toBeNull();
    }
  });

  test("formats North American numbers", () => {
    expect(formatHandle("+15125550148")).toBe("(512) 555-0148");
    expect(formatHandle("+442079460958")).toBe("+442079460958");
  });
});

describe("reminders", () => {
  const now = new Date("2026-09-26T15:00:00Z");
  test("accepts an offset time within a year", () => {
    expect(checkReminderTime("2026-09-27T08:00:00-05:00", now)).toEqual({
      ok: true,
      dueAt: "2026-09-27T13:00:00.000Z",
      dueMs: Date.parse("2026-09-27T13:00:00Z"),
    });
  });
  test("rejects times without an offset, in the past, too soon, or too far", () => {
    expect(checkReminderTime("2026-09-27T08:00:00", now).ok).toBe(false);
    expect(checkReminderTime("tomorrow at 8", now).ok).toBe(false);
    expect(checkReminderTime("2026-09-26T14:00:00Z", now).ok).toBe(false);
    expect(checkReminderTime("2026-09-26T15:04:00Z", now).ok).toBe(false);
    expect(checkReminderTime("2026-09-26T15:06:00Z", now).ok).toBe(true);
    expect(checkReminderTime("2028-01-01T00:00:00Z", now).ok).toBe(false);
  });
  test("cleans stored text", () => {
    expect(cleanStoredText("  a \n\n b  ", 10)).toBe("a b");
    expect(cleanStoredText("x".repeat(20), 5)).toBe("xxxxx");
  });
});

describe("turn input", () => {
  test("carries who, when, and the texts", () => {
    const input = buildTurnInput(
      { ownerName: "Jordan", contactName: "Maya", handle: "+15125550148", timezone: "America/Chicago", now: new Date("2026-09-26T15:04:00Z") },
      ["first", "second"],
    );
    expect(input.split("\n")[0]).toContain("Maya (+15125550148)");
    expect(input).toContain("10:04 AM");
    expect(input.split("\n")[0]).toContain("you assist Jordan");
    expect(input.endsWith("first\nsecond")).toBe(true);
  });
  test("an invalid timezone falls back to UTC", () => {
    expect(formatLocalTime(new Date("2026-09-26T15:04:00Z"), "Not/AZone")).toContain("UTC");
  });
});

describe("relay protocol + transport selection", () => {
  test("parses a relay item", () => {
    expect(
      parseRelayInbound({ guid: "ABC-123", handle: "(512) 555-0148", text: "yo", sentAt: "2026-09-26T15:00:00Z", service: "iMessage" }),
    ).toEqual({
      message: { externalId: "relay:ABC-123", handle: "+15125550148", text: "yo", sentAt: "2026-09-26T15:00:00.000Z", service: "iMessage" },
    });
    expect("skip" in parseRelayInbound({ guid: "../../etc", handle: "+15125550148", text: "x" })).toBe(true);
    expect("skip" in parseRelayInbound({ guid: "A", handle: "chat123", text: "x" })).toBe(true);
    expect("skip" in parseRelayInbound({ guid: "A", handle: "+15125550148", text: "" })).toBe(true);
  });

  test("transport selection and checklist", () => {
    expect(selectedTransport({ IMESSAGE_TRANSPORT: "Relay" })).toBe("relay");
    expect(selectedTransport({ IMESSAGE_TRANSPORT: "twilio" })).toBeNull();
    const list = transportChecklist({ RELAY_TOKEN: "short" }, "relay");
    expect(list).toEqual([expect.objectContaining({ key: "RELAY_TOKEN", ok: false })]);
    expect(JSON.stringify(transportChecklist({ SENDBLUE_API_SECRET: "super-secret-value" }, "sendblue"))).not.toContain(
      "super-secret-value",
    );
  });
});

describe("tool activity", () => {
  test("recognizes refused tool results", async () => {
    const { toolRefusal } = await import("../lib/format");
    expect(toolRefusal('{"scheduled":false,"error":"too soon"}', false)).toBe("too soon");
    expect(toolRefusal('{"scheduled":true}', false)).toBeNull();
    expect(toolRefusal("Unknown tool", true)).toBe("Unknown tool");
  });
});
