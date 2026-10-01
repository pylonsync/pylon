using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Linq;
using System.Net;
using System.Net.Sockets;
using System.Net.WebSockets;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using Pylon;
using Pylon.Realtime;
using Xunit;

namespace Pylon.Tests
{
    public class ShardConnectionTests
    {
        static ShardConnection Make(string shard = "arena-1", Action<ShardConnectionOptions>? tweak = null)
        {
            var o = new ShardConnectionOptions
            {
                BaseUrl = new Uri("http://localhost:4321"),
                SubscriberId = "p1",
                Dispatcher = PylonDispatcher.Inline,
            };
            tweak?.Invoke(o);
            return new ShardConnection(shard, o);
        }

        static byte[] Json(string s) => Encoding.UTF8.GetBytes(s);

        [Fact]
        public void UrlsMatchTheOtherClients()
        {
            Assert.Equal("ws://localhost:4321/shard?shard=arena-1&sid=p1&v=2", Make().BuildUrl().ToString());
            Assert.Equal(
                "wss://app.stack0.app/shard?shard=a%20b&sid=p1&v=2",
                Make("a b", o => o.BaseUrl = new Uri("https://app.stack0.app")).BuildUrl().AbsoluteUri);
            Assert.Equal(
                "ws://localhost:4324/?shard=arena-1&sid=p1&v=2",
                Make(tweak: o => o.WsPort = 4324).BuildUrl().ToString());
            Assert.Equal(
                "ws://edge.test/s?shard=arena-1&sid=p1&v=2",
                Make(tweak: o => o.WsUrl = new Uri("ws://edge.test/s?sid=p1&shard=old&v=2")).BuildUrl().ToString());
        }

        [Fact]
        public void SnapshotsDecodeAndTeachTheInputCodec()
        {
            var c = Make();
            ShardSnapshot? got = null;
            c.Snapshot += s => got = s;
            Assert.Null(c.InputCodec);
            var payload = MessagePack.Encode(PylonValue.Object(("players", PylonValue.Array(PylonValue.Object(("x", 1.5))))));
            c.OnFrame(null, ShardWire.Frame(1, 1, 12, 3, payload), 100);
            Assert.NotNull(got);
            Assert.Equal(12UL, got!.Tick);
            Assert.Equal(3UL, got.Ack);
            Assert.Equal(1.5, got.State!["players"][0]["x"].AsDouble());
            Assert.Equal((byte)ShardCodec.MessagePack, c.InputCodec);
            Assert.Equal(12UL, c.Tick);
            Assert.Equal(3UL, c.Ack);
            Assert.True(c.Clock.Ready);
        }

        [Fact]
        public void SnapshotsInAnUnknownCodecKeepTheirBytes()
        {
            var c = Make();
            ShardSnapshot? got = null;
            c.Snapshot += s => got = s;
            c.OnFrame(null, ShardWire.Frame(1, (byte)ShardCodec.Bincode, 1, 0, new byte[] { 7, 8 }), 0);
            Assert.Null(got!.State);
            Assert.Equal(new byte[] { 7, 8 }, got.Payload);
        }

        [Fact]
        public void ReplicationFramesApplyAndDoNotSetTheInputCodec()
        {
            var c = Make();
            ShardReplicationUpdate? got = null;
            c.Replication += u => got = u;
            var f = new TestFrame { Full = true };
            f.Spawn.Add((5, TestFrame.P(1, 2, 3), null));
            c.OnFrame(null, ShardWire.Frame(3, 4, 9, 0, f.Encode()), 0);
            Assert.NotNull(got);
            Assert.Equal(new ulong[] { 5 }, got!.Summary.Spawned);
            Assert.Equal(2.0, c.Entities.Get(5)!.Y, 5);
            Assert.Null(c.InputCodec);
        }

        [Fact]
        public void ABadReplicationFrameClearsTheTableAndReportsIt()
        {
            var c = Make();
            var errors = new List<Exception>();
            c.Error += errors.Add;
            var f = new TestFrame { Full = true };
            f.Spawn.Add((1, TestFrame.P(), null));
            c.OnFrame(null, ShardWire.Frame(3, 4, 1, 0, f.Encode()), 0);
            var bad = new TestFrame();
            bad.Update.Add((99, TestFrame.P(), TestFrame.P(1), null));
            c.OnFrame(null, ShardWire.Frame(3, 4, 2, 0, bad.Encode()), 0);
            Assert.IsType<ReplicationException>(Assert.Single(errors));
            Assert.Equal(0, c.Entities.Count);
        }

        [Fact]
        public void RejectionsDecode()
        {
            var c = Make();
            ShardInputRejection? got = null;
            c.InputRejected += r => got = r;
            c.OnFrame(null, ShardWire.Frame(2, 0, 1, 7, Json("{\"client_seq\":7,\"code\":\"rate_limited\",\"message\":\"slow\"}")), 0);
            Assert.Equal(7UL, got!.ClientSeq);
            Assert.Equal("rate_limited", got.Code);
        }

        [Fact]
        public void ATransferMovesToTheNewShardAndStartsOver()
        {
            var c = Make();
            var moves = new List<(string To, string From)>();
            c.Transferred += (to, from) => moves.Add((to, from));
            var f = new TestFrame { Full = true };
            f.Spawn.Add((1, TestFrame.P(), null));
            c.OnFrame(null, ShardWire.Frame(3, 4, 5, 0, f.Encode()), 0);
            c.OnFrame(null, ShardWire.Frame(1, 1, 6, 0, MessagePack.Encode(PylonValue.Object())), 0);
            c.OnFrame(null, ShardWire.Frame(4, 0, 7, 0, Json("{\"shard\":\"zone-2\",\"ticket\":\"v1.x.y\"}")), 0);
            Assert.Equal("zone-2", c.ShardId);
            Assert.Equal(new[] { ("zone-2", "arena-1") }, moves);
            Assert.Equal(0, c.Entities.Count);
            Assert.Null(c.InputCodec);
            Assert.False(c.Clock.Ready);
            Assert.Contains("shard=zone-2", c.BuildUrl().Query);

            // The same shard on another machine is not a move for the app.
            c.OnFrame(null, ShardWire.Frame(4, 0, 1, 0, Json("{\"shard\":\"zone-2\",\"ticket\":\"v1.x.z\"}")), 0);
            Assert.Single(moves);
        }

        [Fact]
        public void OnlyAnUnauthorizedPolicyCloseWithTheSameCredentialsIsFinal()
        {
            Assert.True(ShardConnection.RefusedForGood(1008, "unauthorized: ticket expired", false, false));
            Assert.False(ShardConnection.RefusedForGood(1008, "unauthorized: ticket expired", true, false));
            Assert.True(ShardConnection.RefusedForGood(1008, "unauthorized: bad", true, true));
            Assert.False(ShardConnection.RefusedForGood(1008, "shard \"x\" not found", false, false));
            Assert.False(ShardConnection.RefusedForGood(1000, "unauthorized", false, false));
            Assert.False(ShardConnection.RefusedForGood(null, null, false, false));
        }

        [Fact]
        public void TicketsExpireByTheirPayload()
        {
            string Ticket(long exp) =>
                "v1." + Convert.ToBase64String(Encoding.UTF8.GetBytes($"{{\"exp\":{exp},\"shard\":\"a\"}}"))
                    .TrimEnd('=').Replace('+', '-').Replace('/', '_') + ".sig";
            var now = DateTimeOffset.FromUnixTimeSeconds(1_000_000);
            Assert.True(ShardTicket.Expired(Ticket(1_000_003), now));
            Assert.False(ShardTicket.Expired(Ticket(1_000_060), now));
            Assert.False(ShardTicket.Expired("not-a-ticket", now));
            Assert.False(ShardTicket.Expired("v1.!!!.sig", now));
        }

        [Fact]
        public void SendingWhileClosedSendsNothing()
        {
            var c = Make();
            Exception? error = null;
            c.Error += e => error = e;
            Assert.Equal(0UL, c.Send("join"));
            Assert.NotNull(error);
        }

        // ---- against a WebSocket server ----

        [Fact]
        public async Task ConnectsWithHeadersSendsInputsAndFollowsATransfer()
        {
            using var server = new FakeShardServer();
            var inputs = new BlockingCollection<(string Shard, string Text)>();
            server.OnConnection = async (ctx, ws) =>
            {
                var shard = ctx.Request.QueryString["shard"]!;
                if (shard == "arena-1")
                {
                    await ws.SendAsync(ShardWire.Frame(1, 0, 1, 0, Json("{\"n\":1}")), WebSocketMessageType.Binary, true, default);
                    var msg = await FakeShardServer.Receive(ws);
                    inputs.Add((shard, msg.Text!));
                    await ws.SendAsync(ShardWire.Frame(1, 0, 2, 1, Json("{\"n\":2}")), WebSocketMessageType.Binary, true, default);
                    await ws.SendAsync(ShardWire.Frame(4, 0, 3, 1, Json("{\"shard\":\"zone-2\",\"ticket\":\"t2\"}")), WebSocketMessageType.Binary, true, default);
                    await ws.CloseAsync(WebSocketCloseStatus.NormalClosure, "moved", default);
                    return;
                }
                await ws.SendAsync(ShardWire.Frame(1, 1, 1, 0, MessagePack.Encode(PylonValue.Object(("zone", true)))), WebSocketMessageType.Binary, true, default);
                var packed = await FakeShardServer.Receive(ws);
                inputs.Add((shard, MessagePack.Decode(packed.Binary!).ToJson()));
                await FakeShardServer.Receive(ws);
            };

            var snapshots = new BlockingCollection<(string Shard, ShardSnapshot Snap)>();
            var moves = new BlockingCollection<string>();
            using var c = new ShardConnection("arena-1", new ShardConnectionOptions
            {
                WsUrl = server.Url("arena-1"),
                SubscriberId = "p1",
                Token = "tok",
                Ticket = "t1",
                Dispatcher = PylonDispatcher.Inline,
            });
            c.Snapshot += s => snapshots.Add((c.ShardId, s));
            c.Transferred += (to, _) => moves.Add(to);
            c.Connect();

            var first = Take(snapshots);
            Assert.Equal(1, first.Snap.State!["n"].AsInt());
            Assert.Equal(1UL, c.Send(PylonValue.Object(("move_to", PylonValue.Object(("x", 1), ("y", 2))))));
            var input = Take(inputs);
            Assert.Equal("{\"input\":{\"move_to\":{\"x\":1,\"y\":2}},\"client_seq\":1}", input.Text);
            Assert.Equal(1UL, Take(snapshots).Snap.Ack);
            Assert.Equal("zone-2", Take(moves));
            var zone = Take(snapshots);
            Assert.True(zone.Snap.State!["zone"].AsBool());
            Assert.Equal("zone-2", c.ShardId);

            // The first request carried the configured ticket; after the
            // transfer, the transfer's ticket.
            Assert.Equal("Bearer tok", server.Requests[0].Headers["Authorization"]);
            Assert.Equal("t1", server.Requests[0].Headers["X-Pylon-Shard-Ticket"]);
            Assert.Equal("t2", server.Requests[1].Headers["X-Pylon-Shard-Ticket"]);
            Assert.Equal("zone-2", server.Requests[1].QueryString["shard"]);

            // The zone speaks MessagePack, so inputs now go as MessagePack.
            Assert.Equal(2UL, c.Send("join"));
            Assert.Equal("{\"input\":\"join\",\"client_seq\":2}", Take(inputs).Text);
        }

        [Fact]
        public async Task ReconnectsAfterADropButStopsWhenRefused()
        {
            using var server = new FakeShardServer();
            var connections = 0;
            server.OnConnection = async (ctx, ws) =>
            {
                var n = Interlocked.Increment(ref connections);
                if (n == 1)
                {
                    // Drop the TCP connection without a close frame.
                    ws.Abort();
                    ctx.Response.Abort();
                    return;
                }
                await ws.CloseAsync(WebSocketCloseStatus.PolicyViolation, "unauthorized: ticket expired", default);
            };
            var states = new BlockingCollection<(ShardConnectionState State, string? Reason)>();
            using var c = new ShardConnection("arena-1", new ShardConnectionOptions
            {
                WsUrl = server.Url("arena-1"),
                SubscriberId = "p1",
                Ticket = "t1",
                ReconnectBaseDelay = TimeSpan.FromMilliseconds(20),
                Dispatcher = PylonDispatcher.Inline,
            });
            c.StateChanged += (s, r) => states.Add((s, r));
            c.Connect();
            (ShardConnectionState State, string? Reason) s;
            do s = Take(states); while (s.State != ShardConnectionState.Failed);
            Assert.Contains("unauthorized: ticket expired", s.Reason);
            Assert.Equal(2, connections);
            await Task.Delay(200);
            Assert.Equal(2, connections);
        }

        [Fact]
        public async Task ATicketProviderGetsANewTicketAfterARefusal()
        {
            using var server = new FakeShardServer();
            server.OnConnection = async (ctx, ws) =>
            {
                if (ctx.Request.Headers["X-Pylon-Shard-Ticket"] != "fresh-2")
                {
                    await ws.CloseAsync(WebSocketCloseStatus.PolicyViolation, "unauthorized: ticket expired", default);
                    return;
                }
                await ws.SendAsync(ShardWire.Frame(1, 0, 1, 0, Json("{}")), WebSocketMessageType.Binary, true, default);
                await FakeShardServer.Receive(ws);
            };
            var asked = new List<string>();
            var n = 0;
            var opened = new TaskCompletionSource<bool>();
            using var c = new ShardConnection("arena-1", new ShardConnectionOptions
            {
                WsUrl = server.Url("arena-1"),
                SubscriberId = "p1",
                TicketProvider = (shard, _) =>
                {
                    lock (asked) asked.Add(shard);
                    return Task.FromResult("fresh-" + Interlocked.Increment(ref n));
                },
                ReconnectBaseDelay = TimeSpan.FromMilliseconds(10),
                Dispatcher = PylonDispatcher.Inline,
            });
            c.Snapshot += _ => opened.TrySetResult(true);
            c.Connect();
            Assert.True(await Task.WhenAny(opened.Task, Task.Delay(10000)) == opened.Task);
            Assert.Equal(new[] { "arena-1", "arena-1" }, asked);
        }

        [Fact]
        public async Task SubprotocolCredentialsForHostsWithoutHeaders()
        {
            using var server = new FakeShardServer { Subprotocol = "bearer.tok%20x" };
            var done = new TaskCompletionSource<bool>();
            server.OnConnection = async (ctx, ws) =>
            {
                await ws.SendAsync(ShardWire.Frame(1, 0, 1, 0, Json("{}")), WebSocketMessageType.Binary, true, default);
                await FakeShardServer.Receive(ws);
            };
            using var c = new ShardConnection("arena-1", new ShardConnectionOptions
            {
                WsUrl = server.Url("arena-1"),
                SubscriberId = "p1",
                Token = "tok x",
                Ticket = "t 1",
                Credentials = ShardCredentialTransport.Subprotocols,
                Dispatcher = PylonDispatcher.Inline,
            });
            c.Snapshot += _ => done.TrySetResult(true);
            c.Connect();
            Assert.True(await Task.WhenAny(done.Task, Task.Delay(10000)) == done.Task);
            var protocols = server.Requests[0].Headers["Sec-WebSocket-Protocol"]!;
            Assert.Contains("bearer.tok%20x", protocols);
            Assert.Contains("ticket.t%201", protocols);
            Assert.Null(server.Requests[0].Headers["X-Pylon-Shard-Ticket"]);
        }

        [Fact]
        public async Task ASilentConnectionReconnectsAfterTheIdleTimeout()
        {
            using var server = new FakeShardServer();
            var connections = 0;
            server.OnConnection = async (ctx, ws) =>
            {
                Interlocked.Increment(ref connections);
                await ws.SendAsync(ShardWire.Frame(1, 0, 1, 0, Json("{}")), WebSocketMessageType.Binary, true, default);
                // Then silence, with the socket open.
                await Task.Delay(TimeSpan.FromSeconds(30));
            };
            var closes = new BlockingCollection<ShardCloseInfo>();
            using var c = new ShardConnection("arena-1", new ShardConnectionOptions
            {
                WsUrl = server.Url("arena-1"),
                SubscriberId = "p1",
                IdleTimeout = TimeSpan.FromMilliseconds(300),
                ReconnectBaseDelay = TimeSpan.FromMilliseconds(10),
                Dispatcher = PylonDispatcher.Inline,
            });
            c.Closed += closes.Add;
            c.Connect();
            Assert.Contains("no frame for", Take(closes).Reason);
            var deadline = DateTime.UtcNow.AddSeconds(10);
            while (Volatile.Read(ref connections) < 2 && DateTime.UtcNow < deadline) await Task.Delay(20);
            Assert.True(Volatile.Read(ref connections) >= 2);
        }

        [Fact]
        public void AFullSendQueueRefusesTheInput()
        {
            using var link = new ShardConnection.WsLink(new ClientWebSocket(), 10);
            Assert.True(link.Enqueue(new byte[8], true));
            Assert.False(link.Enqueue(new byte[8], true));
        }

        [Fact]
        public async Task InputsFromManyThreadsReachTheServerInSequenceOrder()
        {
            using var server = new FakeShardServer();
            var seqs = new BlockingCollection<ulong>();
            server.OnConnection = async (ctx, ws) =>
            {
                await ws.SendAsync(ShardWire.Frame(1, 0, 1, 0, Json("{}")), WebSocketMessageType.Binary, true, default);
                while (true)
                {
                    var msg = await FakeShardServer.Receive(ws);
                    seqs.Add(PylonValue.Parse(msg.Text!)["client_seq"].AsULong());
                }
            };
            var opened = new TaskCompletionSource<bool>();
            using var c = new ShardConnection("arena-1", new ShardConnectionOptions
            {
                WsUrl = server.Url("arena-1"),
                SubscriberId = "p1",
                Dispatcher = PylonDispatcher.Inline,
            });
            c.Snapshot += _ => opened.TrySetResult(true);
            c.Connect();
            Assert.True(await Task.WhenAny(opened.Task, Task.Delay(10000)) == opened.Task);
            const int threads = 8, each = 200;
            var sent = new ConcurrentBag<ulong>();
            await Task.WhenAll(Enumerable.Range(0, threads).Select(_ => Task.Run(() =>
            {
                for (var i = 0; i < each; i++) sent.Add(c.Send(PylonValue.Object(("n", i))));
            })));
            Assert.DoesNotContain(0UL, sent);
            var got = new List<ulong>();
            for (var i = 0; i < threads * each; i++) got.Add(Take(seqs));
            Assert.Equal(Enumerable.Range(1, threads * each).Select(x => (ulong)x), got);
        }

        [Fact]
        public async Task EventsStopAfterDispose()
        {
            using var server = new FakeShardServer();
            server.OnConnection = async (ctx, ws) =>
            {
                for (var i = 1; i < 1000; i++)
                {
                    await ws.SendAsync(ShardWire.Frame(1, 0, (ulong)i, 0, Json("{}")), WebSocketMessageType.Binary, true, default);
                    await Task.Delay(5);
                }
            };
            var count = 0;
            var c = new ShardConnection("arena-1", new ShardConnectionOptions
            {
                WsUrl = server.Url("arena-1"),
                SubscriberId = "p1",
                Dispatcher = PylonDispatcher.Inline,
            });
            c.Snapshot += _ => Interlocked.Increment(ref count);
            c.Connect();
            while (Volatile.Read(ref count) < 3) await Task.Delay(10);
            c.Dispose();
            var after = Volatile.Read(ref count);
            await Task.Delay(200);
            Assert.True(Volatile.Read(ref count) <= after + 1);
            Assert.Equal(ShardConnectionState.Disconnected, c.State);
        }

        static T Take<T>(BlockingCollection<T> c)
        {
            Assert.True(c.TryTake(out var item, TimeSpan.FromSeconds(10)), "timed out");
            return item!;
        }
    }

    /// <summary>A WebSocket server on HttpListener that runs a script per connection.</summary>
    sealed class FakeShardServer : IDisposable
    {
        readonly HttpListener _listener = new HttpListener();
        readonly int _port;
        public Func<HttpListenerContext, WebSocket, Task> OnConnection = (_, _) => Task.CompletedTask;
        public readonly List<HttpListenerRequest> Requests = new List<HttpListenerRequest>();
        public string? Subprotocol;
        /// <summary>Wait this long before accepting each WebSocket (to race a client against its connect).</summary>
        public TimeSpan AcceptDelay;

        public FakeShardServer()
        {
            var probe = new TcpListener(IPAddress.Loopback, 0);
            probe.Start();
            _port = ((IPEndPoint)probe.LocalEndpoint).Port;
            probe.Stop();
            _listener.Prefixes.Add($"http://127.0.0.1:{_port}/");
            _listener.Start();
            _ = Task.Run(AcceptLoop);
        }

        /// <summary>The app origin: the client connects to /shard on it.</summary>
        public Uri Base => new Uri($"http://127.0.0.1:{_port}");

        public Uri Url(string shard) => new Uri($"ws://127.0.0.1:{_port}/shard?shard={shard}&sid=p1&v=2");

        async Task AcceptLoop()
        {
            while (_listener.IsListening)
            {
                HttpListenerContext ctx;
                try
                {
                    ctx = await _listener.GetContextAsync();
                }
                catch (Exception)
                {
                    return;
                }
                _ = Task.Run(async () =>
                {
                    lock (Requests) Requests.Add(ctx.Request);
                    try
                    {
                        if (AcceptDelay > TimeSpan.Zero) await Task.Delay(AcceptDelay);
                        // "*" echoes the first subprotocol the client asked for.
                        var sp = Subprotocol == "*"
                            ? ctx.Request.Headers["Sec-WebSocket-Protocol"]?.Split(',')[0].Trim()
                            : Subprotocol;
                        var ws = (await ctx.AcceptWebSocketAsync(sp)).WebSocket;
                        await OnConnection(ctx, ws);
                    }
                    catch (Exception)
                    {
                        // The client went away mid-script.
                    }
                });
            }
        }

        public static async Task<(string? Text, byte[]? Binary)> Receive(WebSocket ws)
        {
            var buffer = new byte[65536];
            var ms = new System.IO.MemoryStream();
            while (true)
            {
                var r = await ws.ReceiveAsync(new ArraySegment<byte>(buffer), default);
                if (r.MessageType == WebSocketMessageType.Close) throw new WebSocketException("closed");
                ms.Write(buffer, 0, r.Count);
                if (!r.EndOfMessage) continue;
                return r.MessageType == WebSocketMessageType.Text
                    ? (Encoding.UTF8.GetString(ms.ToArray()), null)
                    : (null, ms.ToArray());
            }
        }

        public void Dispose()
        {
            try
            {
                _listener.Stop();
                _listener.Close();
            }
            catch (Exception)
            {
            }
        }
    }
}
