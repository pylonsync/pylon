# Pylon for C# and Unity

A C# client for [Pylon](https://github.com/pylonsync/pylon) apps. It uses
the same routes and wire format as the TypeScript and Swift clients:

- `PylonClient`: guest, magic-code, and password sign-in, session refresh,
  token storage, server functions, and entity reads and writes.
- `ShardConnection`: realtime shards over WebSocket (wire protocol v2), or
  over WebTransport through a native plugin. It decodes JSON and
  MessagePack snapshots, applies entity replication frames, sends inputs,
  and follows reconnects and transfers.
- `Predictor`, `ShardClock`, and `EntityInterpolator`: client-side
  prediction, a server-tick estimate, and entity interpolation for render
  loops.

The core (`Runtime/`) is plain .NET Standard 2.1 with no Unity references
and no reflection. It runs under IL2CPP and in any .NET app. `Unity/` adds
PlayerPrefs token storage and a `JsonUtility` converter. `Plugins/` holds
the WebTransport plugin for each platform (see [WebTransport](#webtransport)).

## Install

In Unity, open **Window > Package Manager**, click **+**, choose **Install
package from git URL**, and enter:

```
https://github.com/pylonsync/pylon.git?path=/packages/csharp#v0.22.12
```

Any release tag from `v0.22.12` on works. Or add it to `Packages/manifest.json`:

```json
"com.pylonsync.pylon": "https://github.com/pylonsync/pylon.git?path=/packages/csharp#v0.22.12"
```

Unity 2021.3 or later. Supported platforms: Windows, macOS, Linux, iOS, and
Android. WebGL is not supported: it has no `System.Net.WebSockets`.

Outside Unity, compile `Runtime/**/*.cs` into your project (it targets
`netstandard2.1`), or reference `Tests~/Pylon.Core/Pylon.Core.csproj`.

## Sign in and call a function

```csharp
using Pylon;
using Pylon.Unity;

var client = new PylonClient(new PylonClientOptions(new Uri("http://localhost:4321"))
{
    Storage = new PlayerPrefsStorage(),
});

var session = await client.SignInAsGuestAsync();
var join = await client.CallFnAsync("joinArena", PylonValue.Object(("arena", "arena-main")));
string ticket = join["ticket"].AsString();
```

Sign-in methods store the token, and later requests send it. Every
method throws `PylonException`. For an HTTP error, `Status`, `Code` (for
example `UNAUTHENTICATED`), and `ServerMessage` come from the server.

Other sign-in methods:

- `StartMagicCodeAsync(email)` and `VerifyMagicCodeAsync(email, code)`
- `SignInWithPasswordAsync(email, password)`
- `RegisterWithPasswordAsync(email, password)`

Platform accounts (see the server's [native sign-in docs](https://github.com/pylonsync/pylon/blob/main/apps/docs/auth/native.mdx)):

- `SignInWithAppleAsync(idToken, name)`: the identity token from Sign in with Apple. Pass the name on the first sign-in; Apple sends it only then.
- `SignInWithGoogleAsync(idToken)`: the ID token from Google Sign-In.
- `SignInWithSteamAsync(ticketHex)`: a session ticket from `ISteamUser::GetAuthTicketForWebApi`, as hex. The package has no Steamworks dependency:

```csharp
// Facepunch.Steamworks
var ticket = await SteamUser.GetAuthTicketForWebApiAsync("my-game-pylon"); // = PYLON_STEAM_IDENTITY
await client.SignInWithSteamAsync(System.BitConverter.ToString(ticket.Data).Replace("-", ""));
```

To keep a session alive, call `RefreshSessionAsync()`, or call
`StartSessionAutoRefresh(expiresAt)` to refresh an hour before the session
expires.

Entities: `ListAsync`, `ListCursorAsync`, `GetAsync`, `CreateAsync`,
`UpdateAsync`, `DeleteAsync`, `AggregateAsync`, `SearchAsync`. For any
other route, use `RequestAsync`.

## Live queries

```csharp
using var party = client.Live("PartyMember", new LiveQueryOptions
{
    Where = PylonValue.Object(("partyId", partyId)),
    OrderBy = "joinedAt",
});
party.Changed += change =>
{
    foreach (var row in party.Rows) Show(row["name"].AsString());
};
```

- The server pushes each insert, update, and delete the caller may read.
  It applies the entity's read policies and field redaction, so a row
  the caller may not read never arrives.
- `Rows` and `Changed` change on the client's dispatcher (the Unity main
  thread). `Synced` is true once the first pull completed.
- After a reconnect it pulls from its cursor, so no change is lost, and
  then removes rows the server stopped returning.
- Rows come from entities with `sync` on (the default). Writes go
  through server functions or the entity routes; the change comes back
  here. There is no offline write queue.
- Disposing the last query closes the live socket. `client.LiveError`
  reports failed pulls and connects; the engine retries on its own.
- For an app with a sync relay, set `client.Options.Live.UseRelay = true`.

## Values and your own types

Arguments, results, rows, and shard snapshots are `PylonValue`: a JSON or
MessagePack value. Reading a missing key or index gives `PylonValue.Null`,
so `snapshot["players"][0]["x"]` never throws. `AsDouble()`, `AsString()`,
and the other `As*` methods throw when the type is wrong.

To use your own types, pass an `IPylonConverter<T>`. The SDK calls it
instead of reflection, so it works under IL2CPP:

```csharp
var move = PylonConverter.Create<Vector2>(
    v => PylonValue.Object(("x", v.x), ("y", v.y)),
    p => new Vector2(p["x"].AsFloat(), p["y"].AsFloat()));

// Or, for a [Serializable] class, Unity's JsonUtility:
var stats = JsonUtilityConverter<PlayerStats>.Instance;
PlayerStats s = await client.CallFnAsync("getStats", null, stats);
```

## Join a shard

```csharp
using Pylon.Realtime;

var shard = new ShardConnection(join["shardId"].AsString(), new ShardConnectionOptions
{
    BaseUrl = client.BaseUrl,
    SubscriberId = join["subscriberId"].AsString(),
    Ticket = ticket,
    TickRate = 20,
    IdleTimeout = TimeSpan.FromSeconds(5),
});
shard.Snapshot += s => Draw(s.State!);
shard.InputRejected += r => Debug.LogWarning($"{r.Code}: {r.Message}");
shard.Connect();

ulong seq = shard.Send(PylonValue.Object(("move_to", PylonValue.Object(("x", 120), ("y", 64)))));
```

- `Send` wraps the input as `{ input, client_seq }`. The input goes as
  MessagePack for a MessagePack shard and as JSON otherwise. It returns
  the sequence number, or 0 when the connection is not open.
- Every snapshot and replication frame carries `Ack`: the highest sequence
  number the shard has processed. Pass it to `Predictor.Reconcile`.
- A replicating shard fills `shard.Entities` (an `EntityTable`). The
  `Replication` event gives each frame's summary (spawned, updated,
  despawned).
- When the server moves the player to another shard, `Transferred` runs
  and the connection reconnects there with the ticket the server sent.
- A ticket that expires or is refused stops the connection
  (`ShardConnectionState.Failed`). To avoid this, set `TicketProvider` to
  a function that gets a new ticket, for example by calling your join
  function again.
- `IdleTimeout` reconnects when no frame arrives for that long. Set it for
  tick-driven shards only.
- Credentials go as `Authorization` and `X-Pylon-Shard-Ticket` headers. To
  send them as `bearer.` and `ticket.` subprotocols instead, set
  `Credentials = ShardCredentialTransport.Subprotocols`.

## WebTransport

Over a WebSocket, one lost TCP packet holds back every frame behind it.
Over WebTransport (QUIC), entity updates travel as datagrams, so a lost
packet delays only its own update. Set `Transport`:

```csharp
var shard = new ShardConnection(shardId, new ShardConnectionOptions
{
    BaseUrl = client.BaseUrl,
    SubscriberId = subscriberId,
    Ticket = ticket,
    Transport = ShardTransport.Auto,
});
```

- `WebSocket`: the default for `ShardConnection`.
- `WebTransport`: WebTransport only. It stops with an error when the
  plugin is missing or the app does not serve WebTransport.
- `Auto`: the default for `ShardGame`. It uses WebTransport when it opens.
  Otherwise it uses a WebSocket, and keeps using WebSockets on that
  connection. The fallback happens when:
  - the plugin is missing
  - the app does not serve WebTransport
  - UDP is blocked
  - the session does not open within `WebTransportTimeout` (3 s)
  - datagrams stop arriving

The app serves WebTransport when `PYLON_WEBTRANSPORT_PORT` is set on the
server (a UDP port). The client reads the endpoint and its certificate
hashes from `/_pylon/shard/webtransport`. To read them from another URL,
set `WebTransportInfoUrl`. `shard.Transport` tells which transport the
open connection uses.

The plugin (`crates/shard-client-ffi`, a Rust WebTransport client with a
C ABI) ships in `Plugins/` for these platforms:

| Platform | File |
| --- | --- |
| macOS (arm64, x86_64) | `macOS/libpylon_shard_client.dylib` |
| Windows x86_64 | `Windows/x86_64/pylon_shard_client.dll` |
| Linux x86_64 (glibc 2.31 or later) | `Linux/x86_64/libpylon_shard_client.so` |
| iOS (device, simulator) | `iOS/pylon_shard_client.xcframework` |
| Android arm64-v8a, armeabi-v7a | `Android/<abi>/libpylon_shard_client.so` |

Outside Unity, copy the file for your platform next to your executable.

## Threads

Callbacks run on the `SynchronizationContext` of the thread that created
the client or connection. In Unity, create them on the main thread. Every
event then runs on the main thread, and it is safe to touch GameObjects
and read `shard.Entities` in handlers and in `Update`. In a console app
with no context, events run on the network thread. To choose where
callbacks run, pass `Dispatcher = PylonDispatcher.From(...)`.

Dispose the connection and the client in `OnDestroy`. Otherwise their
sockets stay open after you leave Play mode.

## A game's render loop: ShardGame

`ShardGame<TInput>` puts the connection, the clock, interpolation, and
prediction together, as the TypeScript `connectShardGame` does:

```csharp
var game = new ShardGame<Move>(shardId, options, moveConverter);
var me = game.Predict<Vector3>((p, input) => Step(p, input));
game.Replication += u =>
{
    if (u.Entities.Get(myEntityId) is { } e) local = me.Reconcile(new Vector3((float)e.X, (float)e.Y, (float)e.Z), u.Ack);
};
game.Connect();

void Update()
{
    game.Frame(); // places game.Entities at the render tick
    foreach (var e in game.Entities.Values) Place(e.Id, e.X, e.Y, e.Z);
    foreach (var id in game.Left) Remove(id);
    // 0: not sent (the connection is down), so do not predict it.
    if (game.Send(input) != 0) local = Step(local, input);
}
```

- `Frame()` draws entities `InterpolationDelay` behind the shard's
  estimated tick (default 100 ms), and fills `Entered` and `Left`.
- Every predictor made by `Predict` records each input `Send` sends,
  drops inputs the shard refuses, and resets when the connection reopens.
- `Send` stamps each input with the tick the last `Frame` drew
  (`view_tick`), so a shard with lag compensation checks aimed actions
  against what the player saw.
- `Latest` is the newest table, not interpolated. `Tick`, `Ack`, `RttMs`,
  `Connected`, and the connection's events are on the game.

The parts are also usable alone: `EntityInterpolator`, `Predictor`, and
`ShardClock`.

## Sample

**Package Manager > Pylon > Samples > Arena** imports a scene. The scene
signs in as a guest, calls `joinArena`, joins the arena shard, and moves
your player. To run it:

1. Start the server: `pylon dev` in `examples/shard-arena` of this repo.
   To use WebTransport, start it with `PYLON_WEBTRANSPORT_PORT=4324`.
2. Open the scene and press Play. The label shows the transport.

## Tests

The tests in `Tests~` run with the .NET SDK (Unity ships one at
`Unity.app/Contents/Resources/Scripting/DotNetSdk`):

```
dotnet test packages/csharp/Tests~/Pylon.Tests
```

They include the shared fixtures that the TypeScript and Swift clients
also check:

- `packages/realtime/src/replication.fixtures.json`
- `packages/realtime/src/wire.fixtures.json`

Rust writes both files, so the three clients cannot drift from the server
encoders. To run the live tests, start `pylon dev` in
`examples/shard-arena` and set `PYLON_TEST_URL` to its origin.
