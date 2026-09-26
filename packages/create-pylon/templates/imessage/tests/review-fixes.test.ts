// Regression tests for the adversarial review's findings. Each test pins one
// class of hole so it cannot come back.
import { describe, expect, test } from "bun:test";
import claimOutbound from "../functions/claimOutbound";
import fireReminder from "../functions/fireReminder";
import settleOutbound from "../functions/settleOutbound";
import toolSetReminder from "../functions/toolSetReminder";
import { DEFAULT_SETTINGS, NEW_UNKNOWN_CONTACTS_PER_HOUR } from "../lib/gate";
import { MAX_REMINDERS_PER_CONTACT_PER_DAY } from "../lib/reminders";
import { ingestInbound, queueOutbound } from "../lib/store";
import { isDryRun } from "../lib/transport";
import { fakeCtx } from "./fake-ctx";

type Handler = (ctx: unknown, args: Record<string, unknown>) => Promise<any>;
const run = (def: unknown, ctx: unknown, args: Record<string, unknown>) =>
  (def as { handler: Handler }).handler(ctx, args);

const msg = (id: string, handle: string, text: string) => ({
  externalId: `relay:${id}`,
  handle,
  text,
  sentAt: new Date().toISOString(),
  service: "iMessage",
});

describe("dev server cannot text real people by default", () => {
  test("pylon dev implies dry run unless explicitly allowed", () => {
    expect(isDryRun({ PYLON_DEV_MODE: "1" })).toBe(true);
    expect(isDryRun({ PYLON_DEV_MODE: "1", IMESSAGE_ALLOW_DEV_SENDS: "1" })).toBe(false);
    expect(isDryRun({})).toBe(false);
    expect(isDryRun({ IMESSAGE_DRY_RUN: "true" })).toBe(true);
  });

  test("a reply queued on a dev server is recorded as dry_run and never scheduled", async () => {
    const f = fakeCtx({ IMESSAGE_TRANSPORT: "sendblue", PYLON_DEV_MODE: "1" });
    await queueOutbound(f.ctx, { conversationId: "c", contactId: "k", handle: "+15125550148", text: "hi", kind: "owner" });
    expect(f.table("Message")[0].status).toBe("dry_run");
    expect(f.scheduled.length).toBe(0);
  });
});

describe("STOP/START cannot make the number send in a loop", () => {
  test("at most one confirmation of each kind per day", async () => {
    const f = fakeCtx({ IMESSAGE_TRANSPORT: "relay" });
    await f.ctx.db.insert("Contact", { handle: "+15125550163", kind: "phone", status: "allowed", optedOut: false });
    const words = ["STOP", "START", "STOP", "START", "STOP", "START"];
    for (const [i, w] of words.entries()) await ingestInbound(f.ctx, msg(`K${i}`, "+15125550163", w), "relay");
    const out = f.table("Message").filter((m) => m.direction === "out");
    expect(out.length).toBe(2);
    expect(f.table("Contact")[0].optedOut).toBe(false);
  });
});

describe("new senders are bounded", () => {
  test("new unknown contacts per hour are capped", async () => {
    const f = fakeCtx({ IMESSAGE_TRANSPORT: "relay" });
    const now = new Date().toISOString();
    for (let i = 0; i < NEW_UNKNOWN_CONTACTS_PER_HOUR; i++) {
      await f.ctx.db.insert("Contact", { handle: `+1512555${String(1000 + i)}`, status: "unknown", createdAt: now });
    }
    const r = await ingestInbound(f.ctx, msg("N1", "+19995550100", "hi"), "relay");
    expect(r.status).toBe("dropped");
    // An existing contact still gets through.
    await f.ctx.db.insert("Contact", { handle: "+15125550148", kind: "phone", status: "allowed", optedOut: false, createdAt: "2020-01-01T00:00:00Z" });
    expect((await ingestInbound(f.ctx, msg("N2", "+15125550148", "hi"), "relay")).status).toBe("answer");
  });
});

describe("reminders", () => {
  test("a paused assistant does not send reminders", async () => {
    const f = fakeCtx({ IMESSAGE_TRANSPORT: "relay" });
    await f.ctx.db.insert("Settings", { ...DEFAULT_SETTINGS, key: "main", paused: true });
    const contactId = await f.ctx.db.insert("Contact", { handle: "+15125550148", status: "allowed", optedOut: false });
    const conversationId = await f.ctx.db.insert("Conversation", { contactId, handle: "+15125550148" });
    const reminderId = await f.ctx.db.insert("Reminder", { contactId, conversationId, text: "x", status: "scheduled" });
    expect(await run(fireReminder, f.ctx, { reminderId })).toEqual({ sent: false });
    expect(f.table("Message").length).toBe(0);
    expect(f.table("Reminder")[0].status).toBe("cancelled");
  });

  test("a contact cannot get more than the daily reminder cap", async () => {
    const f = fakeCtx({});
    const contactId = await f.ctx.db.insert("Contact", { handle: "+15125550148", status: "allowed" });
    const conversationId = await f.ctx.db.insert("Conversation", { contactId, handle: "+15125550148" });
    const when = new Date(Date.now() + 60 * 60 * 1000).toISOString();
    const results = [];
    for (let i = 0; i < MAX_REMINDERS_PER_CONTACT_PER_DAY + 3; i++) {
      const r = await run(toolSetReminder, f.ctx, { contactId, conversationId, when, text: `r${i}` });
      results.push(r.scheduled);
      // Simulate the reminder firing so it no longer counts as open.
      const row = f.table("Reminder").at(-1);
      if (row && r.scheduled) row.status = "sent";
    }
    expect(results.filter(Boolean).length).toBe(MAX_REMINDERS_PER_CONTACT_PER_DAY);
  });

  test("a reminder cannot target another contact's conversation", async () => {
    const f = fakeCtx({});
    const a = await f.ctx.db.insert("Contact", { handle: "+15125550148", status: "allowed" });
    const b = await f.ctx.db.insert("Contact", { handle: "+15125550163", status: "allowed" });
    const convB = await f.ctx.db.insert("Conversation", { contactId: b, handle: "+15125550163" });
    const when = new Date(Date.now() + 60 * 60 * 1000).toISOString();
    const r = await run(toolSetReminder, f.ctx, { contactId: a, conversationId: convB, when, text: "x" });
    expect(r.scheduled).toBe(false);
    expect(f.scheduled.length).toBe(0);
  });
});

describe("Sendblue delivery leases", () => {
  test("only the lease holder can settle; a taken-over lease is refused", async () => {
    const f = fakeCtx({ IMESSAGE_TRANSPORT: "sendblue" });
    const conversationId = await f.ctx.db.insert("Conversation", { contactId: "k", handle: "+15125550148" });
    for (let i = 0; i < 7; i++) {
      await f.ctx.db.insert("Message", {
        conversationId,
        direction: "out",
        transport: "sendblue",
        status: "queued",
        text: `m${i}`,
        createdAt: `2026-09-26T15:00:0${i}Z`,
      });
    }
    const first = await run(claimOutbound, f.ctx, { conversationId });
    expect(first.messages.map((m: { text: string }) => m.text)).toEqual(["m0", "m1", "m2", "m3", "m4"]);
    // A second job while the lease is live claims nothing (order holds).
    expect((await run(claimOutbound, f.ctx, { conversationId })).messages).toEqual([]);

    // Expire the lease; another job takes it over.
    for (const m of f.table("Message")) if (m.status === "leased") m.leaseUntil = "2000-01-01T00:00:00Z";
    const second = await run(claimOutbound, f.ctx, { conversationId });
    expect(second.messages[0].text).toBe("m0");

    const id = first.messages[0].id;
    expect(await run(settleOutbound, f.ctx, { messageId: id, leaseToken: first.leaseToken, status: "sent" })).toEqual({ ok: false });
    expect(await run(settleOutbound, f.ctx, { messageId: id, leaseToken: second.leaseToken, status: "sent" })).toEqual({ ok: true });
    expect(f.table("Message").find((m) => m.id === id)?.status).toBe("sent");
  });
});
