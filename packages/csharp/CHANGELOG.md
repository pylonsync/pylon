# Changelog

## Unreleased

- Live queries: `client.Live(entity, options)` returns a `LiveQuery` whose `Rows` the server keeps current over the sync routes and the live socket, with `Changed` on the dispatcher. It catches up from its cursor after a reconnect and runs the shared sync conformance scenarios that cover reads (#47).
- `ShardGame<TInput>`, a port of the TypeScript `connectShardGame`: the connection, the clock, interpolated entities, and predictors that record inputs, drop refused ones, and reset when the connection reopens (#45).
- `ShardConnectionOptions.Now` sets the clock for frame arrival and input send times.
- The Arena sample runs on `ShardGame`.
- `SignInWithAppleAsync`, `SignInWithGoogleAsync`, and `SignInWithSteamAsync` call the server's native sign-in routes and store the session (#46).
- WebTransport through a native plugin (#48):
  - `ShardConnectionOptions.Transport` takes `WebSocket` (the default for `ShardConnection`), `WebTransport`, or `Auto` (the default for `ShardGame`).
  - `WebTransportInfoUrl` and `WebTransportTimeout` are new options. `ShardConnection.Transport` gives the transport of the open connection.
  - Over WebTransport, entity updates arrive as datagrams. The client applies a tick when all its datagrams and stream frames are there, and acks the datagrams.
  - Auto uses WebSockets when the session does not open or datagrams stop arriving.
  - `Plugins/` holds the plugin (`crates/shard-client-ffi`) for macOS, Windows x86_64, Linux x86_64, iOS, and Android arm64-v8a and armeabi-v7a.
- The Arena sample has a `transport` field and shows the transport it connected over.

## 0.22.12

First release of the C# and Unity client.

- `PylonClient` supports:
  - guest, magic-code, and password sign-in
  - session refresh and auto-refresh
  - token storage (memory, file, and Unity PlayerPrefs)
  - server functions
  - entity reads and writes, aggregate, and search
- `ShardConnection` uses wire protocol v2 with JSON and MessagePack
  codecs. It supports:
  - tickets in headers or subprotocols
  - ticket providers
  - reconnect with backoff
  - transfers
  - rejections
  - an idle timeout
- `EntityTable` decodes replication frames and datagrams. Ids and
  positions are exact to 64 bits.
- `Predictor`, `ShardClock`, and `EntityInterpolator` are ports of the
  TypeScript helpers.
- The package includes the Arena sample scene.
