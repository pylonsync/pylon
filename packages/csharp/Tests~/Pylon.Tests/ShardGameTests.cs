using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Net.WebSockets;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using Pylon;
using Pylon.Realtime;
using Xunit;

namespace Pylon.Tests
{
    /// <summary>A port of packages/realtime/src/game.test.ts.</summary>
    public class ShardGameTests
    {
        sealed class Move
        {
            public double Dx;
        }

        static readonly IPylonConverter<Move> MoveConverter = PylonConverter.Create<Move>(
            m => PylonValue.Object(("dx", m.Dx)),
            v => new Move { Dx = v["dx"].AsDouble() });

        /// <summary>A server-side socket the test drives by hand, with the client's messages collected.</summary>
        sealed class ServerSide
        {
            public readonly WebSocket Socket;
            public readonly BlockingCollection<string> Received = new BlockingCollection<string>();
            public readonly TaskCompletionSource<bool> Done = new TaskCompletionSource<bool>();

            public ServerSide(WebSocket socket)
            {
                Socket = socket;
            }

            public Task Send(byte[] frame) => Socket.SendAsync(frame, WebSocketMessageType.Binary, true, default);

            public async Task Pump()
            {
                try
                {
                    while (true)
                    {
                        var msg = await FakeShardServer.Receive(Socket);
                        if (msg.Text != null) Received.Add(msg.Text);
                    }
                }
                catch (Exception)
                {
                    Done.TrySetResult(true);
                }
            }
        }

        static byte[] Replication(ulong tick, ulong ack, TestFrame f) =>
            ShardWire.Frame((byte)ShardFrameKind.Replication, (byte)ShardCodec.Replication, tick, ack, f.Encode());

        static byte[] Rejection(ulong tick, ulong ack, ulong clientSeq) =>
            ShardWire.Frame((byte)ShardFrameKind.InputRejected, (byte)ShardCodec.Json, tick, ack,
                Encoding.UTF8.GetBytes($"{{\"client_seq\":{clientSeq},\"code\":\"invalid\",\"message\":\"no\"}}"));

        static void Until(Func<bool> condition, string what)
        {
            var deadline = DateTime.UtcNow.AddSeconds(10);
            while (!condition())
            {
                Assert.True(DateTime.UtcNow < deadline, $"timed out waiting for {what}");
                Thread.Sleep(2);
            }
        }

        static T Take<T>(BlockingCollection<T> c)
        {
            Assert.True(c.TryTake(out var item, TimeSpan.FromSeconds(10)), "timed out");
            return item!;
        }

        [Fact]
        public async Task ARenderLoopDrawsEntitiesBetweenFramesAndPredictionFollowsAcks()
        {
            using var server = new FakeShardServer();
            var sides = new BlockingCollection<ServerSide>();
            server.OnConnection = async (ctx, ws) =>
            {
                var side = new ServerSide(ws);
                sides.Add(side);
                await side.Pump();
            };

            double time = 0;
            using var game = new ShardGame<Move>("zone", new ShardConnectionOptions
            {
                WsUrl = server.Url("zone"),
                SubscriberId = "p1",
                TickRate = 20,
                Now = () => Volatile.Read(ref time),
                ReconnectBaseDelay = TimeSpan.FromMilliseconds(5),
                Dispatcher = PylonDispatcher.Inline,
            }, MoveConverter, new ShardGameOptions { InterpolationDelay = TimeSpan.FromMilliseconds(100) });
            var me = game.Predict<double>((x, input) => x + input.Dx);
            double predicted = 0;
            game.Replication += u =>
            {
                var mine = u.Entities.Get(1);
                if (mine != null) predicted = me.Reconcile(mine.X, u.Ack);
            };
            var opened = 0;
            var rejected = new BlockingCollection<ShardInputRejection>();
            game.Opened += () => Interlocked.Increment(ref opened);
            game.InputRejected += rejected.Add;
            // Registered after the game's and the test's own handlers, so a
            // count means they all ran. Tick changes before the handlers do.
            var applied = 0;
            game.Replication += _ => Interlocked.Increment(ref applied);

            Assert.Equal(-1, game.Frame(0));
            Assert.Equal(0UL, game.Send(new Move { Dx = 1 })); // not open yet

            game.Connect();
            var ws = Take(sides);
            Until(() => game.Connected && Volatile.Read(ref opened) == 1, "open");

            // Tick t is built at t * 50 ms and arrives 20 ms later. Entity 2
            // walks 1 unit per tick.
            async Task Deliver(ServerSide side, ulong tick, ulong ack, TestFrame f)
            {
                Volatile.Write(ref time, tick * 50 + 20);
                var before = Volatile.Read(ref applied);
                await side.Send(Replication(tick, ack, f));
                Until(() => Volatile.Read(ref applied) > before, $"tick {tick}");
            }

            var first = new TestFrame { Full = true };
            first.Spawn.Add((1, TestFrame.P(0), null));
            first.Spawn.Add((2, TestFrame.P(1), null));
            await Deliver(ws, 1, 0, first);
            for (ulong tick = 2; tick <= 10; tick++)
            {
                var f = new TestFrame();
                f.Update.Add((2, TestFrame.P(tick - 1), TestFrame.P(tick), null));
                await Deliver(ws, tick, 0, f);
            }

            // 100 ms (2 ticks) behind the newest tick, halfway between frames.
            var renderTick = game.Frame(10 * 50 + 20 + 25);
            Assert.Equal(8.5, renderTick, 1);
            Assert.Equal(8.5, game.Entities[2].X, 1);
            Assert.Equal(0, game.Entities[1].X, 5);
            Assert.Equal(10, game.Latest.Get(2)!.X, 5);

            // Three inputs; the shard applies the first two, clamping the second.
            var s1 = game.Send(new Move { Dx = 1 });
            var s2 = game.Send(new Move { Dx = 1 });
            var s3 = game.Send(new Move { Dx = 1 });
            Assert.Equal(new ulong[] { 1, 2, 3 }, new[] { s1, s2, s3 });
            Assert.True(PylonValue.Parse("{\"input\":{\"dx\":1},\"client_seq\":1}").Equals(PylonValue.Parse(Take(ws.Received))));
            var clamp = new TestFrame();
            clamp.Update.Add((1, TestFrame.P(0), TestFrame.P(1.5), null));
            await Deliver(ws, 11, 2, clamp);
            Assert.Equal(2.5, predicted, 5);
            Assert.Equal(2UL, game.Ack);
            // Sent at 520 ms, acknowledged by the frame at 570 ms.
            Assert.Equal(50, game.RttMs!.Value, 5);

            // The shard refuses input 3: it drops out of the prediction. A
            // rejection frame moves neither the ack nor the clock.
            var tickBefore = game.Clock.ServerTick(time);
            await ws.Send(Rejection(12, 0, 3));
            Assert.Equal(3UL, Take(rejected).ClientSeq);
            Assert.Equal(2UL, game.Ack);
            Assert.Equal(11UL, game.Tick);
            Assert.Equal(tickBefore, game.Clock.ServerTick(time), 5);
            await Deliver(ws, 12, 3, new TestFrame());
            Assert.Equal(1.5, predicted, 5);
            Assert.Equal(3UL, game.Ack);

            // A reconnect drops inputs sent on the old connection.
            game.Send(new Move { Dx = 5 });
            // Send-only: the pump has a receive pending on this socket.
            await ws.Socket.CloseOutputAsync(WebSocketCloseStatus.NormalClosure, "bye", default);
            var ws2 = Take(sides);
            Until(() => game.Connected && Volatile.Read(ref opened) == 2, "the second open");
            var again = new TestFrame { Full = true };
            again.Spawn.Add((1, TestFrame.P(1.5), null));
            await Deliver(ws2, 13, 0, again);
            Assert.Equal(1.5, predicted, 5);
            Assert.Equal(0, me.Count);

            // Entity 2 was not in the full frame: it leaves at tick 13.
            game.Frame(13 * 50 + 20 + 100);
            Assert.False(game.Entities.ContainsKey(2));
            Assert.Equal(new ulong[] { 2 }, game.Left);
        }

        [Fact]
        public async Task ATransferStartsTheEntitiesOver()
        {
            using var server = new FakeShardServer();
            var sides = new BlockingCollection<ServerSide>();
            server.OnConnection = async (ctx, ws) =>
            {
                var side = new ServerSide(ws);
                sides.Add(side);
                await side.Pump();
            };
            double time = 0;
            using var game = new ShardGame<PylonValue>("zone-1", new ShardConnectionOptions
            {
                WsUrl = server.Url("zone-1"),
                SubscriberId = "p1",
                TickRate = 20,
                Now = () => Volatile.Read(ref time),
                Dispatcher = PylonDispatcher.Inline,
            }, PylonConverter.Value);
            var moves = new BlockingCollection<string>();
            game.Transferred += (to, _) => moves.Add(to);
            var applied = new BlockingCollection<ulong>();
            game.Replication += u => applied.Add(u.Tick);
            game.Connect();
            var ws = Take(sides);
            var f = new TestFrame { Full = true };
            f.Spawn.Add((7, TestFrame.P(1), null));
            Volatile.Write(ref time, 520);
            await ws.Send(Replication(10, 0, f));
            Assert.Equal(10UL, Take(applied));
            game.Frame(700);
            Assert.True(game.Entities.ContainsKey(7));

            await ws.Send(ShardWire.Frame(4, 0, 11, 0, Encoding.UTF8.GetBytes("{\"shard\":\"zone-2\",\"ticket\":\"t2\"}")));
            // The server closes the connection after a transfer frame.
            await ws.Socket.CloseOutputAsync(WebSocketCloseStatus.NormalClosure, "moved", default);
            Assert.Equal("zone-2", Take(moves));
            var ws2 = Take(sides);
            var g = new TestFrame { Full = true };
            g.Spawn.Add((9, TestFrame.P(2), null));
            Volatile.Write(ref time, 1000);
            await ws2.Send(Replication(1, 0, g));
            Assert.Equal(1UL, Take(applied));
            Assert.Equal("zone-2", game.ShardId);
            game.Frame(1200);
            Assert.Equal(new ulong[] { 9 }, new List<ulong>(game.Entities.Keys));
        }
    }
}
