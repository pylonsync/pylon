# @pylonsync/realtime

The Pylon shard client with no framework: the wire protocol, the entity
replication decoder, and the pieces a game's render loop needs.

- `connectShardGame(shardId, options)`: a connection that applies each
  replication frame and places entities for a render frame
  (`game.frame(now)`), with interpolation, a server clock estimate,
  prediction for the local player, the input round trip, and reconnects
  with a full frame.
- `connectShard(shardId, options)`: the connection alone.
- `EntityTable`: applies replication frames.
- `EntityInterpolator`, `ShardClock`, `Predictor`: the parts on their own.

```ts
import { connectShardGame } from "@pylonsync/realtime";

const game = connectShardGame<Input>("zone-1", { subscriberId, ticket, tickRate: 20 });

function frame(now: number) {
  game.frame(now);
  for (const id of game.left) removeMesh(id);
  for (const id of game.entered) addMesh(id);
  for (const e of game.entities.values()) moveMesh(e.id, e.x, e.y, e.z);
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);
```

See the shards guide: https://docs.pylonsync.com/concepts/shards
