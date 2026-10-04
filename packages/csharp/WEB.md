# Unity Web

Unity Web players use the same shards and game rules as native players.
Create `PylonClient`, `ShardConnection`, and `ShardGame<TInput>` on the Unity
main thread. Their public APIs are the same on Web and native.

## Transports

| API | Unity Web | Native / Unity Editor |
| --- | --- | --- |
| Authentication and HTTP | Browser `fetch`, bearer token | `HttpClient` |
| Live queries | Browser WebSocket, sync pull over `fetch` | Native WebSocket, HTTP sync pull |
| `ShardConnection` default | Browser WebSocket | Native WebSocket |
| `ShardGame` default (`Auto`) | Browser WebTransport, then WebSocket fallback | Native WebTransport plugin, then WebSocket fallback |

Set `ShardConnectionOptions.Transport = ShardTransport.WebSocket` to force
WebSockets. No WebTransport server or plugin is required for browser play.
`Auto` checks the browser API and the server endpoint. It falls back when
WebTransport cannot open within `WebTransportTimeout` (default 3 seconds).
It also falls back if replication datagrams stop. An explicit
`WebTransport` selection reports failure instead of using WebSockets.
Inspect `game.Connection.Transport` to see the selected transport.

Browser WebSockets cannot set HTTP authentication headers. The SDK sends
`bearer.<escaped-token>` and `ticket.<escaped-ticket>` subprotocols instead.
It applies this choice automatically, even when `Credentials` is `Headers`.
Credentials do not enter the URL. Do not log credential subprotocols.

`UNITY_WEBGL && !UNITY_EDITOR` excludes `ClientWebSocket`, `HttpClient`,
native WebTransport imports, worker threads, and thread-based timers from
the selected networking implementation. Native plugin import settings
exclude WebGL. JavaScript plugins live in `Plugins/WebGL`. No extra package
is required. The browser loops use Unity's main-thread context. Do not use
`Task.Run`, `.Wait()`, or `.Result` to call these APIs from Web game code.

The transport code leaves replication and game inputs unchanged:

- WebSockets use shard wire version 2.
- WebTransport uses wire version 3, length-prefixed stream messages, and
  replication version 2 datagrams.
- The existing complete-tick assembly and datagram acknowledgements apply
  updates before reconciliation.
- `ShardGame.Frame()` records the rendered tick. `Send()` includes that
  tick as `view_tick`, including for attacks. Call `Frame()` before input
  collection. Keep Camelot's input converter and movement rules unchanged.
- Input sequence numbers and acknowledgements use the existing codec.
  Reconnects reset pending prediction. Transfers keep the existing ticket flow.

Both browser adapters use the same internal transport interfaces as the
native adapters. Future datagram changes belong below `ShardGame`; game
code does not need a second input or replication path.

## Server and hosting setup

1. Use Pylon 0.22.14 or later on the server. Validation used 0.22.17.
2. Serve the game over HTTPS in production. Set `BaseUrl` to the HTTPS API
   origin. The SDK derives WSS URLs for `/shard` and `/api/sync/ws`.
   An HTTPS game page cannot connect to an insecure HTTP/WS server.
   HTTP on localhost is suitable for local tests.
3. Forward WebSocket upgrades on both paths. Preserve
   `Sec-WebSocket-Protocol` on the request and response. Allow long-lived
   connections. A proxy timeout must exceed the game's idle period.
4. For separate game and API origins, set the exact game origin in
   `PYLON_CORS_ORIGIN`, for example `https://play.example.com`. Configure
   `manifest.auth.trustedOrigins` when the app also needs trusted auth
   redirects or cookie authentication. Do not confuse CORS with WebSocket
   origin checks.
5. Allow HTTP preflight requests (`OPTIONS`). Permit the methods used by
   the app and the `Authorization`, `Content-Type`, and `Accept` headers.
   CORS must cover authentication, functions, entity routes, sync pulls,
   and the optional WebTransport discovery endpoint.
6. Apply a WebSocket `Origin` allowlist at the server or reverse proxy if
   deployment policy requires one. This SDK uses explicit tokens. The
   runtime's cookie-origin check does not enforce an origin allowlist for
   bearer/ticket clients. Permit the game origin. Permit missing `Origin`
   for native clients if they share the same endpoint. Keep ticket and
   session authorization enabled; an origin check does not replace it.
7. Allow the API, WSS, and optional WebTransport URLs in the page's CSP
   `connect-src` directive. CSP means Content Security Policy.

HTTP requests use `credentials: "omit"`. Sessions use bearer tokens, not
ambient cookies. Do not set browser-controlled headers such as `Origin`,
`Cookie`, or `User-Agent` in `DefaultHeaders`.

For optional WebTransport, set `PYLON_WEBTRANSPORT_PORT` to a reachable UDP
port. Set `PYLON_WEBTRANSPORT_URL` to the public HTTPS session URL when its
public address differs from the bind address. The SDK reads
`/_pylon/shard/webtransport` for the URL and SHA-256 certificate hashes.
The server's development certificate works through this pinning path in
supported browsers. Production certificates and HTTP/3 routing must meet
the target browser's requirements. A WebSocket-only proxy does not carry
WebTransport. Keep WSS available when UDP or WebTransport is blocked.

## Run the minimal sample

In a Unity 6 project, install the package from a release tag that includes
Unity Web support (see the changelog), or from a local checkout:

```json
"com.pylonsync.pylon": "https://github.com/pylonsync/pylon.git?path=/packages/csharp#v<version>"
```

Install Unity Web Build Support for the same editor version. Import the
**Arena** sample from Package Manager. Its editor commands create a test
scene and build it. They replace `Assets/PylonArena.unity` and
`Assets/PylonSample.mat` in the test project. Use an empty test project.

Start the two local servers in separate terminals:

```sh
cd /absolute/path/to/pylon/examples/shard-arena
pylon dev --port 4431
```

```sh
cd /absolute/path/to/pylon/examples/todo-app
pylon dev --port 4435
```

The Todo server is only for the sample's live-query test. It creates,
updates, and deletes one row owned by its new guest session.

Build from the command line (macOS example):

```sh
UNITY=/Applications/Unity/Hub/Editor/6000.6.3f1/Unity.app/Contents/MacOS/Unity
PYLON_SAMPLE_URL=http://localhost:4431 "$UNITY" \
  -batchmode -quit -projectPath /absolute/path/to/test-project \
  -buildTarget WebGL -executeMethod Pylon.Samples.Arena.ArenaBuild.Web \
  -logFile /tmp/pylon-web-build.log
```

Serve the output, then open `http://localhost:8097`:

```sh
python3 -m http.server 8097 --directory /absolute/path/to/test-project/Build/Web
```

Do not open `index.html` with a `file:` URL. The sample disables compression
for this simple server. For a compressed deployment, configure the correct
content type and content encoding for Unity's build files.

Build a native companion from the same project:

```sh
PYLON_SAMPLE_URL=http://localhost:4431 "$UNITY" \
  -batchmode -quit -projectPath /absolute/path/to/test-project \
  -buildTarget StandaloneOSX -executeMethod Pylon.Samples.Arena.ArenaBuild.Mac \
  -logFile /tmp/pylon-native-build.log
open /absolute/path/to/test-project/Build/PylonArena.app
```

The two players use `arena-main` and `frontier-web`. The Arena scene shows
both players and a predicted target marker. The validation panel shows
frontier entity counts and live-query status. Console lines start with
`PYLON_WEB` and include snapshots, acknowledged moves, prediction, entity
counts, and connection cycles.

Verify these checks:

1. Both players move. Snapshot and acknowledgement counts increase.
2. The target marker moves when an input is sent. Later acknowledgements
   remove that input from prediction.
3. Frontier frame counts increase. Both clients report at least two entities.
4. The panel reports `live query create/update/delete passed`.
5. Disconnect the browser network for at least 10 seconds. Re-enable it.
   Frames and acknowledgements must resume. Use developer tools for this
   check so other applications keep their connections.
6. Click **Disconnect and reconnect frontier** ten times. Wait for new
   frames after each click. The cycle count must increase without duplicate
   players for the same subscriber.
7. Unload or close the Web page. Confirm that the browser closes its sockets.
8. To test WebTransport, restart the arena server with
   `PYLON_WEBTRANSPORT_PORT=4434 pylon dev --port 4431`. Reload both clients.
   To test fallback, stop that server and restart it without the variable.
9. Repeat on each deployment browser. Keep the tab visible during checks.

## Camelot verification

1. Update Camelot's package dependency to the release tag. Do not replace its
   `ShardGame`, input converter, prediction, or combat code.
2. Set `MatchClient.serverUrl` to the server's HTTPS URL for deployment.
   For local tests, use the local HTTP origin.
3. Build the existing main scene for Web in Unity 6 Build Profiles. Build
   the same scene for a native platform. Use the same `matchId`.
4. Join once in the browser and once in the native player. Use separate guest
   sessions. Verify that each player sees the other player's movement.
5. Move, jump, and attack with both weapons. Check acknowledgements, hit
   feedback, and reconciliation corrections. Add artificial latency and
   packet loss as a separate gameplay test.
6. Inspect an outgoing attack envelope. It must contain `input.attack`,
   `client_seq`, and a finite `view_tick` from the last rendered frame.
   Verify the server's hit result against its rewind history under latency.
7. Repeat network loss and scene unload/reload checks. A new scene must not
   retain the old client's tasks or sockets. Camelot's ticket provider
   obtains fresh tickets on reconnect.

The Arena sample has no lag-compensated combat. Use Camelot for that check.
For runtime-created primitive bodies, preserve Unity collider types in the
project's `link.xml` if IL2CPP removes them. This is separate from networking.
The sample includes its own collider preservation file.

## Limits

- Hidden or suspended tabs can stop Unity frames. Browser scheduling can
  delay input, interpolation, timeouts, and reconnects. Do not use a hidden
  tab as a latency benchmark. The server remains authoritative.
- WebSockets are reliable and ordered. Packet loss can delay later frames.
  Browser WebTransport avoids that dependency for replication datagrams.
- Receive queues and WebSocket send buffers have bounds. Overflow closes
  the connection. Automatic reconnect obtains a fresh baseline.
- Browser handshake errors do not expose the HTTP status or detailed TLS
  error to C#. Use the browser console and server logs for diagnosis.
- Browser HTTP cancellation aborts the request. It cannot undo a server
  function that already ran. Use application-level idempotency when needed.
- The WebTransport bridge is optional. Browser support and certificate rules
  vary. The initial release does not depend on it.
- No Web mobile, service-worker, background-tab, or cross-origin production
  deployment certification is claimed by the local tests.

## Automated checks

```sh
node --test packages/csharp/Tests~/Browser/bridge.test.mjs
dotnet build packages/csharp/Tests~/Pylon.Core -p:DefineConstants=UNITY_WEBGL
dotnet test packages/csharp/Tests~/Pylon.Tests
```

With the two servers running and WebTransport enabled on the arena server:

```sh
PYLON_TEST_URL=http://localhost:4431 \
PYLON_SYNC_TEST_URL=http://localhost:4435 \
dotnet test packages/csharp/Tests~/Pylon.Tests
```

The live WebTransport tests require the UDP endpoint. Without it, leave
`PYLON_TEST_URL` unset or filter out `LiveWebTransportTests`. Provider sign-in
tests require their own credentials.

See [validation results](WEB-VALIDATION.md) for the tested versions and gaps.

Unity references: [Web platform limits](https://docs.unity.com/en-us/engine/6000.3/manual/platform-specific/webgl/intro/technical-overview)
and [JavaScript plugins](https://docs.unity.com/en-us/engine/6000.6/manual/platform-specific/webgl/develop/interactingwithbrowserscripting/web-interacting-browser-js).
