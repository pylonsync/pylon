# Pylon for C# and Unity

A C# client for [Pylon](https://github.com/pylonsync/pylon) apps. It uses
the same routes and wire format as the TypeScript and Swift clients:

- `PylonClient`: guest, magic-code, and password sign-in, session refresh,
  token storage, server functions, and entity reads and writes.
- `ShardConnection`: realtime shards over WebSocket (wire protocol v2). It
  decodes JSON and MessagePack snapshots, applies entity replication
  frames, sends inputs, and follows reconnects and transfers.
- `Predictor`, `ShardClock`, and `EntityInterpolator`: client-side
  prediction, a server-tick estimate, and entity interpolation for render
  loops.

The core (`Runtime/`) is plain .NET Standard 2.1 with no Unity references
and no reflection. It runs under IL2CPP and in any .NET app. `Unity/` adds
PlayerPrefs token storage and a `JsonUtility` converter.

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

To keep a session alive, call `RefreshSessionAsync()`, or call
`StartSessionAutoRefresh(expiresAt)` to refresh an hour before the session
expires.

Entities: `ListAsync`, `ListCursorAsync`, `GetAsync`, `CreateAsync`,
`UpdateAsync`, `DeleteAsync`, `AggregateAsync`, `SearchAsync`. For any
other route, use `RequestAsync`.

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

## Threads

Callbacks run on the `SynchronizationContext` of the thread that created
the client or connection. In Unity, create them on the main thread. Every
event then runs on the main thread, and it is safe to touch GameObjects
and read `shard.Entities` in handlers and in `Update`. In a console app
with no context, events run on the network thread. To choose where
callbacks run, pass `Dispatcher = PylonDispatcher.From(...)`.

Dispose the connection and the client in `OnDestroy`. Otherwise their
sockets stay open after you leave Play mode.

## Render loop helpers

```csharp
var interp = new EntityInterpolator();
shard.Replication += u => interp.Record(u.Entities, u.Summary, u.Tick);

void Update()
{
    var renderTick = shard.Clock.ServerTick(ShardClock.Now()) - 0.1 * 20; // 100 ms behind at 20 Hz
    interp.Update(renderTick);
    foreach (var e in interp.Entities.Values) Place(e.Id, e.X, e.Y, e.Z);
}
```

`Predictor<TState, TInput>` replays unacknowledged inputs on top of the
server's state (see the TypeScript `Predictor` for the full pattern).

## Sample

**Package Manager > Pylon > Samples > Arena** imports a scene. The scene
signs in as a guest, calls `joinArena`, joins the arena shard, and moves
your player. To run it:

1. Start the server: `pylon dev` in `examples/shard-arena` of this repo.
2. Open the scene and press Play.

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
