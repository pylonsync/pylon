import { describe, expect, test } from "bun:test";
import { bearerToken, checkSecret, constantTimeEqual, headerValue } from "../lib/secrets";

const TOKEN = "relay-token-0123456789abcdef0123";

describe("relay token auth", () => {
  test("accepts the configured bearer token", () => {
    expect(checkSecret(bearerToken(`Bearer ${TOKEN}`), TOKEN)).toEqual({ ok: true });
    expect(checkSecret(bearerToken(`bearer   ${TOKEN}  `), TOKEN)).toEqual({ ok: true });
  });

  test("rejects a wrong or missing token", () => {
    expect(checkSecret(bearerToken(`Bearer ${TOKEN}0`), TOKEN)).toEqual({ ok: false, reason: "invalid" });
    expect(checkSecret(bearerToken("Bearer nope"), TOKEN)).toEqual({ ok: false, reason: "invalid" });
    expect(checkSecret(bearerToken(TOKEN), TOKEN)).toEqual({ ok: false, reason: "missing" });
    expect(checkSecret(bearerToken(undefined), TOKEN)).toEqual({ ok: false, reason: "missing" });
    expect(checkSecret(bearerToken("Basic abc"), TOKEN)).toEqual({ ok: false, reason: "missing" });
  });

  test("an unset or short server token rejects everything", () => {
    expect(checkSecret("anything", undefined)).toEqual({ ok: false, reason: "not_configured" });
    expect(checkSecret("", "")).toEqual({ ok: false, reason: "not_configured" });
    expect(checkSecret("abc", "abc")).toEqual({ ok: false, reason: "not_configured" });
  });

  test("constantTimeEqual compares full strings", () => {
    expect(constantTimeEqual("same-value", "same-value")).toBe(true);
    expect(constantTimeEqual("same-value", "same-valuf")).toBe(false);
    expect(constantTimeEqual("", "x")).toBe(false);
  });

  test("headerValue is case-insensitive", () => {
    expect(headerValue({ Authorization: "x" }, "authorization")).toBe("x");
    expect(headerValue(null, "authorization")).toBeNull();
  });
});
