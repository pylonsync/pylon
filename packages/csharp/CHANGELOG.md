# Changelog

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
