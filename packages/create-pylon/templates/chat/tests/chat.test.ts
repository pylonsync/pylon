import { describe, expect, test } from "bun:test";
import {
  cleanName,
  dayLabel,
  groupMessages,
  guestHue,
  guestName,
  initials,
  sortMessages,
  typingLabel,
  unreadCount,
  type ChatMessage,
} from "../lib/chat";
import { seedChannels, seedMessages, seedPeople } from "../lib/seed";

const at = (iso: string, authorId = "a", id = iso, channelId = "c1"): ChatMessage => ({
  id,
  channelId,
  authorId,
  text: "x",
  createdAt: iso,
});

describe("guest identity", () => {
  test("the same id always gets the same name and hue", () => {
    expect(guestName("guest_abc")).toBe(guestName("guest_abc"));
    expect(guestHue("guest_abc")).toBe(guestHue("guest_abc"));
  });

  test("names are two words and hues are angles", () => {
    for (const id of ["guest_1", "guest_2", "guest_ffff", "u_42"]) {
      expect(guestName(id)).toMatch(/^[A-Z][a-z]+ [A-Z][a-z]+$/);
      const hue = guestHue(id);
      expect(hue).toBeGreaterThanOrEqual(0);
      expect(hue).toBeLessThan(360);
    }
  });
});

describe("cleanName", () => {
  test("trims and collapses whitespace", () => {
    expect(cleanName("  Maya   Chen ")).toBe("Maya Chen");
  });
  test("removes zero-width and bidi control characters", () => {
    expect(cleanName("Ma\u200bya\u202e")).toBe("Maya");
    expect(cleanName("\u200b\u200b")).toBeNull();
  });
  test("rejects empty and over-long names", () => {
    expect(cleanName("   ")).toBeNull();
    expect(cleanName("x".repeat(33))).toBeNull();
    expect(cleanName("x".repeat(32))).toBe("x".repeat(32));
  });
});

test("initials", () => {
  expect(initials("Maya Chen")).toBe("MC");
  expect(initials("theo")).toBe("T");
  expect(initials("Priya de la Raman")).toBe("PR");
  expect(initials("  ")).toBe("?");
  expect(initials("🎉 Party")).toBe("🎉P");
});

describe("sortMessages", () => {
  test("orders by time and puts unsent rows last", () => {
    const pending: ChatMessage = { id: "p", channelId: "c1", authorId: "a", text: "x" };
    const sorted = sortMessages([at("2026-09-26T10:05:00Z"), pending, at("2026-09-26T10:00:00Z")]);
    expect(sorted.map((m) => m.id)).toEqual(["2026-09-26T10:00:00Z", "2026-09-26T10:05:00Z", "p"]);
  });
});

test("equal timestamps fall back to id order", () => {
  const sorted = sortMessages([
    at("2026-09-26T10:00:00Z", "a", "b-id"),
    at("2026-09-26T10:00:00Z", "a", "a-id"),
  ]);
  expect(sorted.map((m) => m.id)).toEqual(["a-id", "b-id"]);
});

describe("groupMessages", () => {
  test("joins consecutive messages from one author within five minutes", () => {
    const groups = groupMessages([
      at("2026-09-26T10:00:00Z", "a"),
      at("2026-09-26T10:03:00Z", "a"),
      at("2026-09-26T10:04:00Z", "b"),
      at("2026-09-26T10:20:00Z", "b"),
    ]);
    expect(groups.map((g) => [g.authorId, g.messages.length])).toEqual([
      ["a", 2],
      ["b", 1],
      ["b", 1],
    ]);
  });

  test("starts a new group on a new day", () => {
    const groups = groupMessages([
      at(new Date(2026, 8, 25, 23, 58).toISOString(), "a"),
      at(new Date(2026, 8, 26, 0, 1).toISOString(), "a"),
    ]);
    expect(groups).toHaveLength(2);
    expect(groups[0].day).not.toBe(groups[1].day);
  });

  test("a pending row joins the sender's last group", () => {
    const pending: ChatMessage = { id: "p", channelId: "c1", authorId: "a", text: "x", createdAt: null };
    const groups = groupMessages([at("2026-09-26T10:00:00Z", "a"), pending]);
    expect(groups).toHaveLength(1);
    expect(groups[0].messages.map((m) => m.id)).toEqual(["2026-09-26T10:00:00Z", "p"]);
  });
});

describe("dayLabel", () => {
  const now = new Date(2026, 8, 26, 12, 0);
  test("today and yesterday", () => {
    expect(dayLabel(new Date(2026, 8, 26, 8, 0).toISOString(), now)).toBe("Today");
    expect(dayLabel(new Date(2026, 8, 25, 23, 0).toISOString(), now)).toBe("Yesterday");
  });
  test("weekday within a week, full date after", () => {
    expect(dayLabel(new Date(2026, 8, 22, 9, 0).toISOString(), now)).toBe("Tuesday");
    expect(dayLabel(new Date(2026, 8, 14, 9, 0).toISOString(), now)).toBe("Monday, September 14");
    expect(dayLabel(new Date(2025, 8, 14, 9, 0).toISOString(), now)).toBe("Sunday, September 14, 2025");
  });
});

test("typingLabel", () => {
  expect(typingLabel([])).toBe("");
  expect(typingLabel(["Maya"])).toBe("Maya is typing");
  expect(typingLabel(["Maya", "Theo"])).toBe("Maya and Theo are typing");
  expect(typingLabel(["Maya", "Theo", "Priya"])).toBe("Maya, Theo, and Priya are typing");
  expect(typingLabel(["Maya", "Theo", "Priya", "Jonas"])).toBe("Maya, Theo, and 2 others are typing");
});

describe("unreadCount", () => {
  const msgs = [
    at("2026-09-26T10:00:00Z", "other", "1"),
    at("2026-09-26T10:05:00Z", "other", "2"),
    at("2026-09-26T10:06:00Z", "me", "3"),
    at("2026-09-26T10:07:00Z", "other", "4", "c2"),
  ];
  test("counts other people's messages after the mark in that channel", () => {
    expect(unreadCount(msgs, "c1", "2026-09-26T10:01:00Z", "me")).toBe(1);
  });
  test("a channel without a mark counts as read", () => {
    expect(unreadCount(msgs, "c1", undefined, "me")).toBe(0);
  });
});

describe("seed", () => {
  test("every seeded message points at a seeded channel and person", () => {
    const channels = new Set(seedChannels.map((c) => c.slug));
    const people = new Set(seedPeople.map((p) => p.userId));
    for (const m of seedMessages) {
      expect(channels.has(m.channel)).toBe(true);
      expect(people.has(m.author)).toBe(true);
    }
  });
  test("seed ids cannot collide with session user ids", () => {
    for (const p of seedPeople) expect(p.userId.startsWith("seed_")).toBe(true);
  });
});
