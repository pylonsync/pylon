# __APP_NAME__

Lumen, a streaming AI assistant built with [Pylon](https://pylonsync.com).
Visitors chat right away in a guest session. Signing in moves their chats to
the account. Conversations sync live across tabs and devices, and the provider
key stays on the server.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321. With no provider key, the app shows the setup
steps, seeds three example conversations (labeled as examples and read-only),
and still stores the messages you send.

## Add a model provider

```bash
# .env
PYLON_LLM_PROVIDER=anthropic
ANTHROPIC_API_KEY=sk-ant-...
```

Restart `pylon dev`. For OpenAI, set `PYLON_LLM_PROVIDER=openai` and
`OPENAI_API_KEY`, then replace the ids in `chat.models` in
`lib/site.config.ts`. For an OpenAI-compatible gateway such as OpenRouter,
also set `PYLON_LLM_BASE_URL` (for example `https://openrouter.ai/api`) and use
the gateway's model ids.

The model list in `lib/site.config.ts` is the allowlist. `app.ts` passes it to
`llm({ allowedModels })`, and `respond` refuses any other id.

## How it works

- **Replies.** `functions/respond.ts` is a streaming action. It calls
  `ctx.llm.stream` and writes each token to `ctx.stream`. The client reads it
  with `streamFn("respond", …)`.
- **Other tabs.** `_beginTurn` stores the call's resumable stream id on the
  assistant message. Another tab (or this one after a reload) attaches with
  `resumeStream(streamId)` and shows the same tokens. The final text is written
  to the message row, which syncs everywhere.
- **Stop and regenerate.** Stop stores the text shown so far with status
  `stopped`. Regenerate removes the last reply and writes a new one.
- **Guests.** `<EnsureGuest>` creates a guest session on first visit. Every
  function the chat calls accepts guests (`auth: "guest"`).
- **Sign-in keeps history.** Before signing in, the guest gets a one-time code
  from `prepareGuestClaim`. After sign-in, `claimGuestHistory` uses it to move
  the guest's conversations to the account.
- **Limits.** `repliesPerHour` in `lib/site.config.ts` caps replies per rolling
  hour, lower for guests than for accounts.

## Privacy

`Conversation` and `Message` rows are readable only by their owner. Clients can
rename their own conversations; every other write goes through a server
function that checks ownership. `GuestClaim` is never readable by clients, and
only a hash of each claim code is stored. `User.passwordHash` is `serverOnly`.

## Rebrand it

The name, colors, system prompt, suggestions, models, and limits live in
`lib/site.config.ts`. Replace or delete the examples in `lib/examples.ts`.

## Layout

```
app.ts                     entities, policies, llm() allowlist, fonts
lib/site.config.ts         brand, prompt, suggestions, models, limits
lib/chat.ts                history, titles, day groups, error copy (tested)
lib/markdown.ts            Markdown parser for replies (tested)
functions/respond.ts       streams a reply with ctx.llm.stream
app/(chat)/chat-client.tsx the only file that talks to Pylon on the client
components/chat/           sidebar, thread, composer, setup card
app/login/                 optional sign-in
```

## Test

```bash
pylon test
```

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
