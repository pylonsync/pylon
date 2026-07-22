# __APP_NAME__

A streaming AI chat app built with [Pylon](https://pylonsync.com). One server
handles token streaming, conversation history, and cross-tab sync.

Tokens stream from the built-in `POST /api/ai/stream` endpoint, so your provider
API key never reaches the browser. Conversations are sync-backed and
owner-scoped; a chat sent in one tab appears in the other.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321. You can create chats immediately. Configure an LLM
provider to receive replies, then open a second tab to see conversations and
messages sync.

## Configure the model

The assistant replies after you configure a provider. Without one, the app
still boots and displays a configuration notice:

```bash
# .env
PYLON_AI_PROVIDER=anthropic        # or "openai" / "custom"
PYLON_AI_API_KEY=sk-ant-...
PYLON_AI_MODEL=claude-sonnet-4-6   # default when none is picked
```

`/api/ai/stream` is auth-gated (it uses your signed-in session) and rate-limited
per user, so a drive-by caller can't burn your budget.

### Switching models / providers

The composer has a **model picker** — its options live in `lib/site.config.ts`
(`chat.models`, each with a provider label), and the chosen id is sent per
request. Client-chosen models must be allow-listed server-side:

```bash
PYLON_AI_MODELS_ALLOWED=claude-sonnet-4-6,claude-opus-4-8,gpt-4o
```

`/api/ai/stream` talks to **one** provider. To offer models from **multiple
providers** in one picker, route through an OpenAI-compatible gateway like
[OpenRouter](https://openrouter.ai):

```bash
PYLON_AI_PROVIDER=custom
PYLON_AI_BASE_URL=https://openrouter.ai/api/v1
PYLON_AI_API_KEY=<openrouter key>
```

then set `chat.models` + `PYLON_AI_MODELS_ALLOWED` to the gateway's slugs
(`anthropic/claude-sonnet-4`, `openai/gpt-4o`, `google/gemini-2.5-pro`, …).

## How it works

- **Streaming.** `app/chat-client.tsx` POSTs the conversation to
  `/api/ai/stream` and reads the SSE response (`data: {choices:[{delta:…}]}` …
  `data: [DONE]`), appending tokens to the live assistant bubble.
- **Realtime history.** `Conversation` + `Message` are owner-scoped entities
  read with `db.useQuery` — private to each user and synced across their tabs.
  Messages are written with optimistic `db.insert` (userId is stamped from the
  session via `field.owner()`), so no custom write functions are needed.
- **Sign-in required.** Chats are tied to your account, so the home page
  redirects unauthenticated visitors to `/login`. Once signed in, your history
  follows you across tabs and devices.

## Privacy

`Conversation` and `Message` policies are owner-scoped (`auth.userId ==
data.userId`) — you can only ever read or write your own. `User.passwordHash` is
`serverOnly`. The provider key lives only on the server.

## Rebrand it

The name, colors, system prompt, empty-state copy, and starter prompts live in
**`lib/site.config.ts`**.

## Layout

```
app.ts                 Conversation + Message (owner-scoped) + User
lib/site.config.ts     brand + system prompt + suggestions (edit this)
app/page.tsx           renders the chat island
app/chat-client.tsx    sidebar + thread + streaming via /api/ai/stream
app/login/             optional sign-in (carries history across devices)
```

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
