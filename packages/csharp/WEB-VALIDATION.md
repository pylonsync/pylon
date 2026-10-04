# Unity Web validation

Results for the Unity Web support described in [WEB.md](WEB.md). Unity
6000.6.3f1, Pylon server 0.22.17, macOS 26 on Apple silicon. The Arena
sample ran against `examples/shard-arena` (port 4471, WebTransport on UDP
4474) and `examples/todo-app` (port 4435).

## Browsers

Each run loaded the Arena Web sample for 45 seconds, restarted the arena
server, and ran 30 seconds more. "Checks" means:

- guest sign-in, `joinArena` and `joinFrontier` function calls;
- arena movement: snapshots and acknowledged moves increase, and the
  predicted target follows each sent input;
- frontier replication: frames increase;
- live query: a row is created, updated and deleted, and each change
  reaches `LiveQuery`;
- after the server restart, the shards reconnect with fresh tickets and
  snapshots and acknowledgements resume.

| Browser | Transport | Result |
| --- | --- | --- |
| Firefox (Playwright build, desktop) | WebTransport | All checks passed |
| Firefox | WebSocket (server without WebTransport) | All checks passed |
| WebKit 26.6 (Playwright build) | WebTransport | All checks passed |
| WebKit 26.6 | WebSocket (server without WebTransport) | All checks passed |
| Safari 26.4 | WebTransport | Connected; snapshots, acknowledgements, frontier frames and live query passed. Not put through the server restart. |
| Chromium (Playwright build) | WebTransport | All checks passed |
| Chrome, Edge (installed) | WebTransport and WebSocket | Checked during development: movement, acknowledgements, live query, network loss in Chrome, and ten disconnect/reconnect cycles of the frontier shard. |

A Firefox player and a native macOS player built from the same project
shared `arena-main` (the panel showed 2 players and 2 frontier entities).
Camelot checked lag-compensated combat on v0.22.18. A Chrome player and a
native macOS player fought in one local match over WebSocket (the server
ran no WebTransport, so the SDK fell back):

- The browser sent 742 inputs. All 17 attack envelopes carried
  `input.attack`, a `client_seq`, and a finite `view_tick`.
- The native player landed 14 bow hits on the browser player. The browser
  player's largest movement correction was 1.1 cm.
- After 12 seconds offline, the browser reconnected and resumed inputs. A
  page reload closed the old socket, and a new guest joined.

The Arena sample has no combat; use a game like Camelot for that check.

## Automated checks

- `node --test packages/csharp/Tests~/Browser/bridge.test.mjs`: 13 passed
  (receive limits, datagram backlog, close codes kept, failed connections,
  release, fetch abort).
- `dotnet build packages/csharp/Tests~/Pylon.Core -p:DefineConstants=UNITY_WEBGL`:
  the browser code paths compile with warnings as errors.
- `dotnet test packages/csharp/Tests~/Pylon.Tests`: 159 passed, 16 skipped
  without servers; 171 passed, 4 skipped (provider sign-in) with both
  servers and WebTransport running.

## Not tested

- Mobile browsers (iOS Safari, Android Chrome).
- Windows and Linux desktop browsers.
- A deployed HTTPS origin with a production certificate and a reverse
  proxy. All runs used `http://localhost`.
- Hidden or background tabs, where browsers slow or stop Unity frames.
- Playwright's offline emulation does not cut WebSockets in Firefox, so
  network loss was tested by restarting the server instead.
