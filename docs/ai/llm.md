# Pylon: `ctx.llm`, a provider-abstracted LLM client

A server-side LLM client for `ctx.llm.complete(...)` calls from
mutations and actions. Configure it once at boot from env, then use the same
call shape for Anthropic, OpenAI, or a custom HTTPS endpoint set through the
base-URL override.

## Quick start

```bash
# .env
PYLON_LLM_PROVIDER=anthropic
ANTHROPIC_API_KEY=sk-ant-...
PYLON_LLM_MODEL=claude-sonnet-4-5
```

```ts
// functions/draftReply.ts
import { action } from "@pylonsync/functions";

export default action({
  args: { ticketId: v.id("Ticket") },
  async handler(ctx, { ticketId }) {
    const ticket = await ctx.db.get("Ticket", ticketId);
    const resp = await ctx.llm.complete({
      system: "You draft empathetic, concise support replies.",
      messages: [
        { role: "user", content: `Ticket body:\n${ticket.body}` },
      ],
    });
    const text = resp.content.find((b) => b.type === "text")?.text ?? "";
    await ctx.db.update("Ticket", ticketId, { draftReply: text });
  },
});
```

## Why a Pylon primitive

`ctx.llm.complete` adds these controls to direct Anthropic or OpenAI calls:

- **Single wire shape** across providers. Code written against
  Anthropic Messages content blocks (text + tool_use + tool_result)
  works unchanged against OpenAI; the transport translates.
- **Server-side API key** that never reaches the browser. Functions
  call the host runtime, which holds the key.
- **Built-in rate limit** + per-user model allowlist (same `PYLON_AI_*`
  envs as `/api/ai/stream`).
- **Auditable usage.** Every call passes through the runtime's
  request trace + metrics path.
- **Tool-use loops first-class.** The content-block shape lets you
  feed `tool_use` blocks back as `tool_result` blocks in the next
  call without translating per provider.

## Configuration

The runtime resolves provider + key in this order:

1. `PYLON_LLM_PROVIDER` env (`anthropic` | `openai`) +
   the corresponding key (`ANTHROPIC_API_KEY` /
   `OPENAI_API_KEY`).
2. Legacy `PYLON_AI_PROVIDER` + `PYLON_AI_API_KEY` (kept for
   compat with the existing `/api/ai/stream` endpoint).
3. Auto-detect: `ANTHROPIC_API_KEY` → anthropic; otherwise
   `OPENAI_API_KEY` → openai.

When none of these are set, `ctx.llm.complete` rejects with
`LLM_NOT_CONFIGURED` so the gap surfaces in logs instead of
silently no-op'ing.

### app.ts declaration

The manifest can pin defaults that travel with the bundle:

```ts
import { llm } from "@pylonsync/sdk";

export default {
  // ...
  llm: llm({
    provider: "anthropic",
    defaultModel: "claude-sonnet-4-5",
    allowedModels: ["claude-sonnet-4-5", "claude-haiku-4-5"],
  }),
};
```

Env takes precedence when set, so operators can override per deploy
without changing the bundle.

### Env reference

| Env                          | Purpose                                                   |
| ---------------------------- | --------------------------------------------------------- |
| `PYLON_LLM_PROVIDER`         | `anthropic` \| `openai`. Auto-detected from keys if unset. |
| `ANTHROPIC_API_KEY`          | Used when provider is anthropic.                          |
| `OPENAI_API_KEY`             | Used when provider is openai.                             |
| `PYLON_LLM_MODEL`            | Default model when caller doesn't supply `model`.         |
| `PYLON_LLM_BASE_URL`         | Override the provider host (Together, Groq, local proxy). |
| `PYLON_LLM_MAX_TOKENS_CAP`   | Cap applied to `max_tokens` on every request.             |
| `PYLON_AI_MODELS_ALLOWED`    | Comma-separated allowlist for caller-supplied `model`. Merged with manifest `llm({ allowedModels })`. |
| `PYLON_AI_RATE_LIMIT_MAX`    | Per-user requests in window. Default 30.                  |
| `PYLON_AI_RATE_LIMIT_WINDOW` | Window in seconds. Default 3600 (1h).                     |

## Wire shape

`ctx.llm.complete` accepts the Anthropic Messages shape:

```ts
{
  model?: string;          // overrides default; subject to allowlist
  messages: LlmMessage[];
  system?: string;
  tools?: LlmTool[];
  max_tokens?: number;     // default 4096
  temperature?: number;
}
```

A message is `{ role, content }` where `content` is either a string
(plain text) or an array of content blocks:

```ts
type LlmContentBlock =
  | { type: "text"; text: string }
  | { type: "tool_use"; id: string; name: string; input: object }
  | { type: "tool_result"; tool_use_id: string; content: string; is_error?: boolean };
```

The response:

```ts
{
  model: string;
  content: LlmContentBlock[];
  stop_reason: "end_turn" | "tool_use" | "max_tokens" | "stop_sequence";
  usage: { input_tokens: number; output_tokens: number };
}
```

OpenAI requests translate at the transport boundary. `tool_use`
blocks emit as `tool_calls`, `tool_result` blocks emit as
`role: "tool"` follow-ups, and the response shape comes back in the
canonical Anthropic format.

## Tool-use loop

The pattern for agent-style tool use:

```ts
const tools = [
  {
    name: "lookup_customer",
    description: "Find a customer by email.",
    input_schema: {
      type: "object",
      properties: { email: { type: "string" } },
      required: ["email"],
    },
  },
];

let messages: LlmMessage[] = [
  { role: "user", content: "Who is alice@example.com?" },
];

for (let turn = 0; turn < 10; turn++) {
  const resp = await ctx.llm.complete({ system, tools, messages });
  messages = [...messages, { role: "assistant", content: resp.content }];

  if (resp.stop_reason !== "tool_use") break;

  const toolResults = [];
  for (const block of resp.content) {
    if (block.type !== "tool_use") continue;
    if (block.name === "lookup_customer") {
      const customer = await ctx.db.lookup(
        "Customer",
        "email",
        block.input.email as string,
      );
      toolResults.push({
        type: "tool_result" as const,
        tool_use_id: block.id,
        content: JSON.stringify(customer),
      });
    }
  }
  messages.push({ role: "user", content: toolResults });
}
```

Cap the loop count. A runaway model that keeps requesting tools
will burn through quota.

## HTTP routes

`POST /api/llm/complete` is non-streaming and uses the same wire shape as
`ctx.llm.complete`. Authenticated callers only; rate-limited per
user with the same `PYLON_AI_*` envs. Browser clients hit this
when they want the full response in one shot.

`POST /api/ai/stream` uses SSE streaming. It emits OpenAI-style chunks
(`data: {choices:[{delta:{content:"..."}}]}`) for compatibility
with existing clients. Use this for progressive UI.

## Error codes

| Code                        | HTTP | Meaning                                                          |
| --------------------------- | ---- | ---------------------------------------------------------------- |
| `LLM_NOT_CONFIGURED`        | 503  | No provider + key wired at boot.                                 |
| `LLM_REQUIRES_AUTH`         | -    | `ctx.llm.complete` called from a public/anonymous handler. Elevate first. |
| `LLM_NOT_AVAILABLE_IN_QUERY`| -    | `ctx.llm.complete` called from a query handler. Move to mutation/action. |
| `MODEL_OVERRIDE_FORBIDDEN`  | 403  | Caller supplied `model` but no allowlist is configured.          |
| `MODEL_NOT_ALLOWED`         | 403  | Caller's `model` isn't in env or manifest allowlist.             |
| `RATE_LIMITED`              | 429  | Per-user limit hit. Retry after `retry_after_secs`.              |
| `PROVIDER_HTTP_<n>`         | 502  | Provider returned an error; `n` is their status code. Body is server-side only. |
| `PROVIDER_UNREACHABLE`      | 504  | Network failure connecting to the provider.                      |
| `INVALID_REQUEST`           | 400  | Request body didn't deserialize.                                 |
| `INVALID_RESPONSE`          | 500  | Provider returned a body we couldn't parse.                      |

Provider error bodies are redacted before reaching the caller. Any
substring that looks like an API key (`sk-ant-*`, `sk-proj-*`,
`sk-*`) is replaced with `<redacted>` before egress.

## Limits

- Query handlers cannot use `ctx.llm`. Reactive re-runs
  on dep invalidation would silently re-bill the LLM call and
  violate the reactive purity contract. Put LLM calls in mutations
  or actions; from a query, call `ctx.runAction("doLlmThing", {...})`
  via an action wrapper.
- Anonymous calls reject with
  `LLM_REQUIRES_AUTH` when `auth.userId` is null and the caller
  isn't admin. Webhook receivers that pass HMAC verification
  must call `ctx.auth.elevate({ admin: true, reason: "..." })`
  before reaching for `ctx.llm`.
- Persist chat-history message arrays in your own entities; `ctx.llm` is
  stateless.
- For background fan-out, schedule
  an action via `ctx.scheduler.runAfter(0, "draftReply", { ... })`
  and call `ctx.llm.complete` inside the action.
- `ctx.llm.complete` uses request/response rather than streaming. Use
  `POST /api/ai/stream` from the browser
  when you need progressive output, or wait for `ctx.llm.stream(...)`
  in a future release.
