# __APP_NAME__

A team chat on [Pylon](https://pylonsync.com): channels, live messages,
presence, and typing indicators, served from one synced backend.

## Develop

```bash
__RUN_DEV__
```

Open http://localhost:4321. The first visit loads a demo workspace: three
channels and a conversation between four people. To see realtime with a second
person, open the app in a private window (a normal second tab shares your guest
session, so it counts as the same person). Messages, typing, the online count,
unread badges, name changes, and deletes all update in the other window at once.

## Layout

```
app.ts                        data model: Channel, Member, Message + policies + fonts
app/page.tsx                  "/" (`?c=<slug>` opens a channel)
app/chat-client.tsx           client island: guest session, live queries, presence, unread marks
components/chat/              sidebar, message list, composer, avatar
functions/joinChat.ts         seeds the demo on first boot, creates the caller's Member row
functions/renameMember.ts     changes the caller's display name
lib/chat.ts                   pure helpers: guest names, grouping, day labels, unread counts
lib/seed.ts                   the demo channels, people, and messages
tests/                        pure-logic and component tests (`pylon test`)
```

## How it works

- **No sign-up.** `<EnsureGuest>` mints a guest session. `joinChat` gives each
  guest a name like "Amber Otter" and an avatar color, derived from the user id.
  Click the pencil next to your name to change it.
- **Optimistic send.** The composer calls `db.insert("Message", …)`. The row
  paints at once, marked "Sending", and gets its server timestamp a moment later.
- **The server decides who and when.** `authorId: field.owner()` stamps the
  sender from the session and rejects anyone else's id. The insert policy
  requires a session, an existing channel, and no client-supplied `createdAt`.
  Only the author can delete a message; nobody can edit one.
- **Live everywhere.** `db.useQuery` is a live subscription: new messages,
  renames, and deletes reach every open window with no polling.
- **Presence.** `useRoom` joins one presence room. The online list and the
  "is typing" line come from it. Nothing about presence is stored.
- **Delete** asks for confirmation. On touch screens the delete button is
  always shown on your own messages.
- **Unread badges** are per browser: the last message you saw in each channel
  is kept in `localStorage`.

## Make it yours

- Edit the channels and demo conversation in `lib/seed.ts`. Set
  `includeDemoConversation = false` to start with empty channels.
- **Real accounts:** email/password is built in. Swap `<EnsureGuest>` for
  `<SignedIn>` / `<SignedOut>` from `@pylonsync/client`.
- **Private channels:** add a `ChannelMember` entity and change the Message
  read policy to
  `exists(ChannelMember where channelId == data.channelId and userId == auth.userId)`.
- **Long history:** every client syncs every message. For a busy workspace, add
  a sync window to the Message entity or page older history with
  `db.useInfiniteQuery`.

## Deploy

```bash
pylon deploy
```

Docs: https://docs.pylonsync.com
