import { describe, expect, test } from "bun:test";
import {
  buildSendblueRequest,
  parseSendblueInbound,
  SENDBLUE_SEND_URL,
  sendblueConfig,
  sendblueTransport,
  verifySendblueWebhook,
} from "../lib/sendblue";

const SECRET = "test-webhook-secret-0123456789abcdef";

describe("Sendblue webhook verification", () => {
  test("accepts the configured secret", () => {
    expect(verifySendblueWebhook({ "sb-signing-secret": SECRET }, SECRET)).toEqual({ ok: true });
  });

  test("header name is case-insensitive", () => {
    expect(verifySendblueWebhook({ "SB-Signing-Secret": SECRET }, SECRET)).toEqual({ ok: true });
  });

  test("rejects a wrong secret", () => {
    expect(verifySendblueWebhook({ "sb-signing-secret": `${SECRET}x` }, SECRET)).toEqual({
      ok: false,
      reason: "invalid",
    });
    expect(verifySendblueWebhook({ "sb-signing-secret": SECRET.slice(0, -1) }, SECRET)).toEqual({
      ok: false,
      reason: "invalid",
    });
  });

  test("rejects a missing header", () => {
    expect(verifySendblueWebhook({}, SECRET)).toEqual({ ok: false, reason: "missing" });
    expect(verifySendblueWebhook({ "sb-signing-secret": "  " }, SECRET)).toEqual({ ok: false, reason: "missing" });
    expect(verifySendblueWebhook(undefined, SECRET)).toEqual({ ok: false, reason: "missing" });
  });

  test("fails closed when no secret (or a short one) is configured", () => {
    expect(verifySendblueWebhook({ "sb-signing-secret": "" }, "")).toEqual({ ok: false, reason: "not_configured" });
    expect(verifySendblueWebhook({ "sb-signing-secret": "short" }, "short")).toEqual({
      ok: false,
      reason: "not_configured",
    });
  });
});

describe("Sendblue inbound parsing", () => {
  const base = {
    accountEmail: "you@example.com",
    content: "Hello!",
    is_outbound: false,
    status: "RECEIVED",
    message_handle: "99DCC379-DD76-4712-BA65-11EFB33B8CD6",
    from_number: "+19998887777",
    to_number: "+15122164639",
    date_sent: "2025-12-12T15:41:20.932Z",
    service: "iMessage",
  };

  test("normalizes a received message", () => {
    const r = parseSendblueInbound(base, "+15122164639");
    expect(r).toEqual({
      message: {
        externalId: "sendblue:99DCC379-DD76-4712-BA65-11EFB33B8CD6",
        handle: "+19998887777",
        text: "Hello!",
        sentAt: "2025-12-12T15:41:20.932Z",
        service: "iMessage",
      },
    });
  });

  test("skips outbound status events, groups, other numbers, and empty bodies", () => {
    expect("skip" in parseSendblueInbound({ ...base, is_outbound: true }, "")).toBe(true);
    expect("skip" in parseSendblueInbound({ ...base, group_id: "g1" }, "")).toBe(true);
    expect("skip" in parseSendblueInbound(base, "+15125550000")).toBe(true);
    expect("skip" in parseSendblueInbound({ ...base, content: "  " }, "")).toBe(true);
    expect("skip" in parseSendblueInbound({ ...base, message_handle: "" }, "")).toBe(true);
    expect("skip" in parseSendblueInbound({ ...base, from_number: "chat123456" }, "")).toBe(true);
    expect("skip" in parseSendblueInbound("not an object", "")).toBe(true);
  });

  test("an attachment without text becomes a placeholder", () => {
    const r = parseSendblueInbound({ ...base, content: "", media_url: "https://cdn.example/x.jpg" }, "");
    expect("message" in r && r.message.text).toBe("[sent an attachment]");
  });
});

describe("Sendblue outbound", () => {
  const config = sendblueConfig(
    {
      SENDBLUE_API_KEY: "key-id",
      SENDBLUE_API_SECRET: "key-secret",
      SENDBLUE_FROM_NUMBER: "(512) 555-0100",
      SENDBLUE_WEBHOOK_SECRET: SECRET,
    },
    false,
  );

  test("builds the documented request", () => {
    const { url, init } = buildSendblueRequest(config, { messageId: "m1", to: "+15125550148", text: "hi" });
    expect(url).toBe(SENDBLUE_SEND_URL);
    expect(init.method).toBe("POST");
    const headers = init.headers as Record<string, string>;
    expect(headers["sb-api-key-id"]).toBe("key-id");
    expect(headers["sb-api-secret-key"]).toBe("key-secret");
    expect(JSON.parse(String(init.body))).toEqual({ number: "+15125550148", from_number: "+15125550100", content: "hi" });
  });

  test("send reports the provider message handle", async () => {
    const calls: string[] = [];
    const transport = sendblueTransport(config, async (url) => {
      calls.push(url);
      return new Response(JSON.stringify({ status: "QUEUED", message_handle: "abc" }), { status: 200 });
    });
    expect(await transport.send({ messageId: "m1", to: "+15125550148", text: "hi" })).toEqual({
      status: "sent",
      externalId: "sendblue:abc",
    });
    expect(calls).toEqual([SENDBLUE_SEND_URL]);
  });

  test("send reports provider errors", async () => {
    const transport = sendblueTransport(config, async () =>
      new Response(JSON.stringify({ status: "ERROR", error_message: "bad number" }), { status: 200 }),
    );
    expect(await transport.send({ messageId: "m1", to: "+15125550148", text: "hi" })).toEqual({
      status: "failed",
      error: "Sendblue: bad number",
    });
  });

  test("dry run never calls the network", async () => {
    let called = false;
    const transport = sendblueTransport({ ...config, dryRun: true }, async () => {
      called = true;
      return new Response("{}");
    });
    expect(await transport.send({ messageId: "m1", to: "+15125550148", text: "hi" })).toEqual({ status: "dry_run" });
    expect(called).toBe(false);
  });
});
