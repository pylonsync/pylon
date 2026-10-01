using System;
using System.Collections.Concurrent;
using System.Linq;
using System.Threading.Tasks;
using Pylon;
using Pylon.Realtime;
using Xunit;

namespace Pylon.Tests
{
    /// <summary>
    /// Against a running server: <c>pylon dev</c> in examples/shard-arena,
    /// with <c>PYLON_TEST_URL</c> set to its origin (for example
    /// <c>http://localhost:4321</c>).
    /// </summary>
    public class LiveArenaTests
    {
        const string Env = "PYLON_TEST_URL";

        static PylonClient NewClient() => new PylonClient(new PylonClientOptions(new Uri(Environment.GetEnvironmentVariable(Env)!))
        {
            Dispatcher = PylonDispatcher.Inline,
        });

        static ShardConnection Connect(PylonClient client, string shard, string subscriber, string ticket) =>
            new ShardConnection(shard, new ShardConnectionOptions
            {
                BaseUrl = client.BaseUrl,
                SubscriberId = subscriber,
                Ticket = ticket,
                Dispatcher = PylonDispatcher.Inline,
                IdleTimeout = TimeSpan.FromSeconds(5),
            });

        static T Take<T>(BlockingCollection<T> c, Func<T, bool> until, int seconds = 15)
        {
            var deadline = DateTime.UtcNow.AddSeconds(seconds);
            while (DateTime.UtcNow < deadline)
            {
                if (c.TryTake(out var item, TimeSpan.FromMilliseconds(200)) && until(item)) return item;
            }
            throw new TimeoutException("the expected event did not arrive");
        }

        [LiveFact(Env)]
        public async Task AGuestSignsInCallsAFunctionAndMovesInTheArena()
        {
            using var client = NewClient();
            var session = await client.SignInAsGuestAsync();
            var me = await client.MeAsync();
            Assert.Equal(session.UserId, me.UserId);

            // Fixed names: the example's kinds cap their instances and keep
            // idle shards running, so a name per run would use them up.
            const string arena = "arena-cs";
            var join = await client.CallFnAsync("joinArena", PylonValue.Object(("arena", arena)));
            Assert.Equal(arena, join["shardId"].AsString());
            var subscriber = join["subscriberId"].AsString();

            var snapshots = new BlockingCollection<ShardSnapshot>();
            var errors = new BlockingCollection<Exception>();
            using var shard = Connect(client, arena, subscriber, join["ticket"].AsString());
            shard.Snapshot += snapshots.Add;
            shard.Error += errors.Add;
            shard.Connect();

            // The first snapshot teaches the codec (MessagePack); then join.
            Take(snapshots, _ => true);
            Assert.True(shard.Send("join") > 0);
            PylonValue Me(ShardSnapshot s) =>
                s.State!["players"].Items.FirstOrDefault(p => p["id"].AsStringOr(null) == subscriber) ?? PylonValue.Null;
            var joined = Take(snapshots, s => !Me(s).IsNull);
            var x0 = Me(joined)["x"].AsDouble();

            var seq = shard.Send(PylonValue.Object(("move_to", PylonValue.Object(("x", 700.0), ("y", 400.0)))));
            var moved = Take(snapshots, s => s.Ack >= seq && Math.Abs(Me(s)["x"].AsDouble() - x0) > 20);
            Assert.Equal(700.0, Me(moved)["tx"].AsDouble(), 3);
            Assert.True(shard.RttMs.HasValue);
            Assert.Empty(errors);
        }

        [LiveFact(Env)]
        public async Task FrontierEntitiesReplicate()
        {
            using var client = NewClient();
            await client.SignInAsGuestAsync();
            const string frontier = "frontier-cs";
            var join = await client.CallFnAsync("joinFrontier", PylonValue.Object(("frontier", frontier), ("size", 500)));
            var updates = new BlockingCollection<ShardReplicationUpdate>();
            using var shard = Connect(client, frontier, join["subscriberId"].AsString(), join["ticket"].AsString());
            shard.Replication += updates.Add;
            shard.Connect();
            // Other players may already be on the shard: this player's entity
            // is the one that appears after the join.
            var before = new System.Collections.Generic.HashSet<ulong>(Take(updates, _ => true).Entities.Entities.Keys);
            shard.Send("join");
            ulong mine = 0;
            Take(updates, u =>
            {
                foreach (var id in u.Entities.Entities.Keys)
                {
                    if (!before.Contains(id)) mine = id;
                }
                return mine != 0;
            });
            var e = shard.Entities.Get(mine)!;
            var start = (e.X, e.Y);
            shard.Send(PylonValue.Object(("move_to", PylonValue.Object(("x", 10.0), ("y", 10.0)))));
            Take(updates, u => u.Entities.Get(mine) is { } now && (now.X, now.Y) != start);
        }

        [LiveFact(Env)]
        public async Task AShardGameInterpolatesFrontierEntities()
        {
            using var client = NewClient();
            await client.SignInAsGuestAsync();
            const string frontier = "frontier-cs";
            var join = await client.CallFnAsync("joinFrontier", PylonValue.Object(("frontier", frontier), ("size", 500)));
            using var game = new ShardGame<PylonValue>(frontier, new ShardConnectionOptions
            {
                BaseUrl = client.BaseUrl,
                SubscriberId = join["subscriberId"].AsString(),
                Ticket = join["ticket"].AsString(),
                TickRate = 20,
                Dispatcher = PylonDispatcher.Inline,
            }, PylonConverter.Value);
            var updates = new BlockingCollection<ShardReplicationUpdate>();
            game.Replication += updates.Add;
            game.Connect();
            Take(updates, _ => true);
            Assert.True(game.Send("join") > 0);
            Take(updates, u => u.Entities.Count > 0);
            // A few frames later, the render tick trails the newest tick and the entities are placed.
            Take(updates, u => u.Tick > game.Tick - 1 && game.Clock.Ready);
            System.Threading.Thread.Sleep(200);
            var renderTick = game.Frame();
            Assert.True(renderTick > 0, $"render tick {renderTick}");
            Assert.True(renderTick < game.Tick, $"render tick {renderTick} should trail tick {game.Tick}");
            Assert.NotEmpty(game.Entities);
        }

        [LiveFact(Env)]
        public async Task ATransferMovesTheConnectionToTheNewZone()
        {
            using var client = NewClient();
            await client.SignInAsGuestAsync();
            const string from = "zone-cs-a";
            const string to = "zone-cs-b";
            var join = await client.CallFnAsync("joinZone", PylonValue.Object(("zone", from)));
            await client.CallFnAsync("joinZone", PylonValue.Object(("zone", to)));

            var snapshots = new BlockingCollection<(string Shard, ShardSnapshot Snap)>();
            var moves = new BlockingCollection<string>();
            using var shard = Connect(client, from, join["subscriberId"].AsString(), join["ticket"].AsString());
            shard.Snapshot += s => snapshots.Add((shard.ShardId, s));
            shard.Transferred += (next, _) => moves.Add(next);
            shard.Connect();
            Take(snapshots, _ => true);
            shard.Send("join");
            Take(snapshots, s => s.Snap.Ack >= 1);

            await client.CallFnAsync("moveZone", PylonValue.Object(("from", from), ("to", to)));
            Assert.Equal(to, Take(moves, _ => true));
            Take(snapshots, s => s.Shard == to);
            Assert.Equal(to, shard.ShardId);
        }

        [LiveFact(Env)]
        public async Task ServerErrorsArriveAsPylonExceptions()
        {
            using var client = NewClient();
            var e = await Assert.ThrowsAsync<PylonException>(() => client.CallFnAsync("joinArena"));
            Assert.Equal(PylonErrorKind.Http, e.Kind);
            Assert.InRange(e.Status, 401, 403);
            Assert.NotNull(e.Code);
        }
    }
}
