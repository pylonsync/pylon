import { expect, test } from "bun:test";
import { providerStatus } from "../lib/provider";

test("no keys means not configured", () => {
  expect(providerStatus({})).toEqual({ configured: false, provider: null });
});

test("a bare Anthropic or OpenAI key selects that provider", () => {
  expect(providerStatus({ ANTHROPIC_API_KEY: "k" })).toEqual({ configured: true, provider: "anthropic" });
  expect(providerStatus({ OPENAI_API_KEY: "k" })).toEqual({ configured: true, provider: "openai" });
});

test("an explicit provider needs its own key or PYLON_AI_API_KEY", () => {
  expect(providerStatus({ PYLON_LLM_PROVIDER: "openai", ANTHROPIC_API_KEY: "k" })).toEqual({
    configured: false,
    provider: "openai",
  });
  expect(providerStatus({ PYLON_AI_PROVIDER: "anthropic", PYLON_AI_API_KEY: "k" })).toEqual({
    configured: true,
    provider: "anthropic",
  });
});

test("blank keys and unknown providers are not configured", () => {
  expect(providerStatus({ ANTHROPIC_API_KEY: "  " }).configured).toBe(false);
  expect(providerStatus({ PYLON_LLM_PROVIDER: "custom", ANTHROPIC_API_KEY: "k" }).configured).toBe(false);
});

test("the result never carries key material", () => {
  const out = JSON.stringify(providerStatus({ ANTHROPIC_API_KEY: "sk-ant-secret" }));
  expect(out).not.toContain("secret");
});
