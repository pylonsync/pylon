# __APP_NAME__

An AI assistant people text over iMessage, built on Pylon. You choose who it
answers; everything it says and does shows up live in your dashboard.

```bash
cp .env.example .env     # set PYLON_ADMIN_EMAILS, a model key, and a transport
bun install
bun run dev              # http://localhost:4321
```

Sign in at `/login` with the email in `PYLON_ADMIN_EMAILS`. In `pylon dev` the
six-digit code appears on the page; in production it is emailed.

`pylon dev` never sends texts: replies are stored and marked "Dry run". A dev
server listens on your local network and returns sign-in codes in its
responses, so anyone on the network who knows your email could sign in as you.
Set `IMESSAGE_ALLOW_DEV_SENDS=1` only on a network you trust. The dashboard
opens with three sample threads (marked "Sample"); remove them from Setup once
real texts arrive.

## How it works

```
 iPhone ──iMessage──▶ Sendblue ──webhook──────────▶ sendblueWebhook ─┐
                      your Mac (relay/) ──relaySync─▶ relaySync ──────┤
                                                                      ▼
                     ingestInbound: dedupe → allowlist → rate limit → store
                                                                      │
                                              processTurn (job) ◀─────┘
                                                   │  runs agent() as the owner
                                                   ▼
                     assistant: history + tools (notes, reminders)
                                                   │
                     reply → split into bubbles → queued → transport → iPhone
```

- **Transports** (`lib/transport.ts`). One `MessageTransport` interface, two
  adapters. `IMESSAGE_TRANSPORT=sendblue` uses Sendblue's API
  (`lib/sendblue.ts`). `IMESSAGE_TRANSPORT=relay` uses the Bun script in
  `relay/` on a Mac you own.
- **Allowlist** (`lib/gate.ts`). Only contacts marked Allowed get answers.
  Anyone else is stored under Requests and ignored, or gets one fixed reply if
  you turn that on. Blocked contacts are never answered. STOP and START work
  for everyone.
- **Rate limit.** Each contact gets at most N answered texts per rolling hour
  (Setup, default 30). A number that is not on the allowlist can store at most
  20 texts per hour, and at most 50 new numbers are stored per hour; the rest
  are dropped. STOP and START confirmations go out at most once a day each.
  The assistant can create at most 10 reminders per contact per day, each at
  least five minutes out, and none fire while the assistant is paused.
- **The agent** (`functions/assistant.ts`) is Pylon's `agent()`: the tool loop,
  the transcript in the synced `AgentRun`/`AgentMessage` entities, and token
  streaming come from the framework. One run per conversation holds its
  history; after 80 turns a new run starts with the last few texts as context.
- **Tools.** `remember_note`, `recall_notes`, `forget_note`, `set_reminder`,
  `list_reminders`, `cancel_reminder`. They act only on the person being
  answered: the contact comes from the server (the run's context and
  `functions/resolveTurn.ts`), never from the model. Reminders run on Pylon's job scheduler
  (`ctx.scheduler.runAt` → `functions/fireReminder.ts`).
- **Replies** are converted from Markdown to plain text and split into up to
  four bubbles along paragraph breaks (`lib/chunk.ts`).
- **Dashboard.** Owner-only. Every entity policy is `auth.isAdmin`, and the
  owner is the account whose verified email is in `PYLON_ADMIN_EMAILS`.
  Anonymous visitors and other accounts read nothing.

### How a turn runs the agent

`agent()` stores each run under a user. A webhook has no user, so
`processTurn` (an internal action) calls `ctx.agents.run("assistant", ...)` with
`as: { userId: <owner>, admin: true }`. The run belongs to the owner and shows
up live in the dashboard. The call also passes `context: { conversationId,
turnId }`; the tools read it from their `run` argument, and `resolveTurn`
maps it to the contact only while that turn is running. Only server code can
set a run's context.

## Transport 1: Sendblue

No Mac needed. Sendblue hosts the iMessage line.

1. Create an account and a number at [dashboard.sendblue.com](https://dashboard.sendblue.com).
2. Set in `.env` (or `pylon secrets set` on Stack0 Cloud):
   ```
   IMESSAGE_TRANSPORT=sendblue
   SENDBLUE_API_KEY=...
   SENDBLUE_API_SECRET=...
   SENDBLUE_FROM_NUMBER=+15125550100
   SENDBLUE_WEBHOOK_SECRET=<openssl rand -hex 32>
   ```
3. In Sendblue, set the receive webhook to
   `https://<your app>/api/webhooks/sendblueWebhook` with the same secret.
   Sendblue sends it back in the `sb-signing-secret` header on every delivery;
   the webhook compares it in constant time and rejects anything else. Because
   that secret is not bound to the body, Sendblue's `message_handle` is also
   stored as a unique key, so a replayed or retried delivery is stored once.
4. Add your own number on the Allowlist page and text the Sendblue number.

## Transport 2: Mac relay

Uses your Mac and Apple ID. The relay reads new messages from Messages'
database and sends replies through Messages.app.

1. On the server: `IMESSAGE_TRANSPORT=relay` and `RELAY_TOKEN=<openssl rand -hex 32>`.
2. On a Mac that stays on and is signed in to Messages, copy this project and
   run `bun install`. Create `relay/.env` from `relay/.env.example`:
   ```
   PYLON_URL=https://<your app>
   RELAY_TOKEN=<same value as the server>
   ```
3. **Full Disk Access.** System Settings → Privacy & Security → Full Disk
   Access → add the app that runs the relay (Terminal, iTerm, or the `bun`
   binary if you run it under launchd). The relay opens
   `~/Library/Messages/chat.db` read-only and never writes to it.
4. **Automation.** Start it with `bun run relay`. The first reply makes macOS
   ask whether it may control Messages; allow it. You can change this later
   under Privacy & Security → Automation.
5. Keep it running. With launchd, a `~/Library/LaunchAgents` plist that runs
   `bun relay/main.ts` from the project directory with `KeepAlive` works well.

`RELAY_TOKEN` is as sensitive as your Apple ID: whoever holds it can post
texts as any sender and read every reply waiting to go out. Keep it only in
the server environment and in `relay/.env` on the Mac.

The relay forwards every one-to-one text your Apple ID receives, including
texts from numbers not on the allowlist (for example sign-in codes from
services that text from a full phone number). They are stored under Requests
and never answered. Short-code senders are dropped.

How the relay works (`relay/main.ts`):

- It starts at the newest message, so your existing history is never sent to
  the server. Its position (a chat.db ROWID) is saved in
  `~/.pylon-imessage-relay/state.json` and only advances past messages the
  server accepted.
- Every `POLL_MS` (default 3s, minimum 2.5s) it makes one call to `/api/fn/relaySync` with
  `Authorization: Bearer <RELAY_TOKEN>`: new messages in, acks for sent replies,
  and the next replies to send back. The server compares the token in constant
  time.
- Group chats, your own messages, and reactions are skipped. Messages that
  macOS stores only in `attributedBody` (common since macOS 13) are decoded.
- Replies are sent with `osascript`. The message text and the recipient are
  passed as arguments to a fixed AppleScript (`on run argv`), never inserted
  into script source, so a message cannot change what the script does.
- A reply is recorded as sent on disk before it is acknowledged, so a crash
  does not send it twice. Unacknowledged replies return to the queue after 60
  seconds and are marked failed after 5 attempts.

## Environment

| Variable | Needed for |
|---|---|
| `PYLON_ADMIN_EMAILS` | The owner's email. Required. |
| `ANTHROPIC_API_KEY` (or `PYLON_LLM_PROVIDER=openai` + `OPENAI_API_KEY`) | Replies. Without it texts are stored and marked "waiting for setup". The model id is `ASSISTANT_MODEL` in `lib/assistant.ts`. |
| `IMESSAGE_TRANSPORT` | `sendblue` or `relay`. |
| `SENDBLUE_*` | The Sendblue transport. |
| `RELAY_TOKEN` | The relay transport. At least 24 characters. |
| `IMESSAGE_DRY_RUN=1` | Store replies without sending them (both transports). Always on in `pylon dev` unless `IMESSAGE_ALLOW_DEV_SENDS=1`. |
| `APP_URL` | Production: the public origin the Setup page uses for the webhook and relay URLs. |

The Setup page shows which of these are set (never their values), when the
last text came in, and when the relay last checked in.

## Tests

```bash
bun test tests
```

Covers webhook verification (valid, wrong, missing, unconfigured secret),
idempotency, the allowlist and rate limit, STOP/START, relay token auth,
AppleScript argument passing (including a real `osascript` echo run on macOS
with quotes, backslashes, newlines, and unicode), the chat.db reader against a
fixture database built in a temp directory, the agent turn and its tool
fence, and reply chunking. Nothing in the suite sends a message or calls Sendblue.

## Deploy

```bash
pylon deploy
pylon secrets set PYLON_ADMIN_EMAILS=you@example.com ANTHROPIC_API_KEY=... \
  IMESSAGE_TRANSPORT=sendblue SENDBLUE_API_KEY=... SENDBLUE_API_SECRET=... \
  SENDBLUE_FROM_NUMBER=+1... SENDBLUE_WEBHOOK_SECRET=... \
  APP_URL=https://<your app>
```

Sign-in codes in production need an email provider (`PYLON_EMAIL_PROVIDER`);
Stack0 Cloud has one configured.
