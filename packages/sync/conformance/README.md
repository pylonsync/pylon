# Sync engine conformance scenarios

One scenario per file. Both engines run every file:

- TypeScript: `packages/sync/src/conformance.test.ts`
- Swift: `packages/swift/Tests/PylonSyncTests/SyncConformanceTests.swift`

A scenario is a list of steps against a fake server that starts empty with
`seq = 0` and one signed-in user. Server-side seeds take the next seq in
order (1, 2, 3, …). Live frames carry the seq you write.

Steps:

| op | fields | meaning |
|----|--------|---------|
| `seed` | `entity`, `row_id`, `kind`, `data?` | append a server change at the next seq |
| `pull` | | `engine.pull()` |
| `frame` | `frame` | deliver one live WebSocket text frame |
| `update` | `entity`, `id`, `data` | `engine.update` (optimistic + push) |
| `delete` | `entity`, `id` | `engine.delete` (optimistic + push) |
| `expectRow` | `entity`, `id`, `present`, `fields?` | row presence and field values |
| `expectCount` | `entity`, `count` | rows in the local replica |
| `expectCursor` | `last_seq` | engine cursor |

A push applies on the fake server at the next seq, so a `delete` step is
followed by a `pull` when the scenario needs the server's own event in the
replica before asserting.
