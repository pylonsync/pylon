// Whether the server has a model provider configured, derived from the same
// environment variables Pylon reads for `ctx.llm`:
//
//   PYLON_LLM_PROVIDER (or PYLON_AI_PROVIDER) = "anthropic" | "openai"
//   ANTHROPIC_API_KEY / OPENAI_API_KEY (or PYLON_AI_API_KEY)
//
// With no provider set, an ANTHROPIC_API_KEY selects Anthropic and an
// OPENAI_API_KEY selects OpenAI. The result carries no key material.

export type ProviderName = "anthropic" | "openai";

export interface ProviderStatus {
  configured: boolean;
  provider: ProviderName | null;
}

type Env = Record<string, string | undefined>;

function present(env: Env, key: string): boolean {
  const v = env[key];
  return typeof v === "string" && v.trim().length > 0;
}

export function providerStatus(env: Env): ProviderStatus {
  const explicit = (env.PYLON_LLM_PROVIDER || env.PYLON_AI_PROVIDER || "").trim();
  if (explicit === "anthropic") {
    const ok = present(env, "ANTHROPIC_API_KEY") || present(env, "PYLON_AI_API_KEY");
    return { configured: ok, provider: "anthropic" };
  }
  if (explicit === "openai") {
    const ok = present(env, "OPENAI_API_KEY") || present(env, "PYLON_AI_API_KEY");
    return { configured: ok, provider: "openai" };
  }
  if (explicit !== "") return { configured: false, provider: null };
  if (present(env, "ANTHROPIC_API_KEY")) return { configured: true, provider: "anthropic" };
  if (present(env, "OPENAI_API_KEY")) return { configured: true, provider: "openai" };
  return { configured: false, provider: null };
}
