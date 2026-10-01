using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Diagnostics;
using System.Linq;
using System.Net;
using System.Net.Sockets;
using System.Threading;
using System.Threading.Tasks;
using Pylon;
using Pylon.Realtime;
using Xunit;
using Xunit.Abstractions;

namespace Pylon.Tests
{
    /// <summary>
    /// The native WebTransport plugin against a running server:
    /// <c>PYLON_WEBTRANSPORT_PORT=4474 pylon dev</c> in examples/shard-arena,
    /// with <c>PYLON_TEST_URL</c> set to its origin. Needs the plugin for
    /// this host in packages/csharp/Plugins.
    /// </summary>
    public class LiveWebTransportTests
    {
        const string Env = "PYLON_TEST_URL";
        readonly ITestOutputHelper _out;

        public LiveWebTransportTests(ITestOutputHelper output)
        {
            _out = output;
        }

        static PylonClient NewClient() => new PylonClient(new PylonClientOptions(new Uri(Environment.GetEnvironmentVariable(Env)!))
        {
            Dispatcher = PylonDispatcher.Inline,
        });

        /// <summary>A frontier player: (connection, its entity id), with events on a queue this thread drains.</summary>
        sealed class Player : IDisposable
        {
            public readonly ShardConnection Shard;
            readonly ConcurrentQueue<Action> _queue;
            public readonly List<string> Errors = new List<string>();
            public int Opened;

            public Player(ShardConnection shard, ConcurrentQueue<Action> queue)
            {
                Shard = shard;
                _queue = queue;
                Shard.Error += e => Errors.Add(e.Message);
                Shard.Opened += () => Opened++;
            }

            public void Pump()
            {
                while (_queue.TryDequeue(out var a)) a();
            }

            public void Until(string what, Func<bool> done, int seconds = 15)
            {
                var deadline = DateTime.UtcNow.AddSeconds(seconds);
                while (true)
                {
                    Pump();
                    if (done()) return;
                    if (DateTime.UtcNow > deadline)
                        throw new TimeoutException(
                            $"timed out waiting for {what} (tick {Shard.Tick}, ack {Shard.Ack}, {Shard.Entities.Count} entities, " +
                            $"{Shard.Transport}); errors: {string.Join("; ", Errors)}");
                    Thread.Sleep(2);
                }
            }

            public void Dispose() => Shard.Dispose();
        }

        static async Task<Player> JoinFrontier(PylonClient client, string frontier, Action<ShardConnectionOptions> tweak,
            Action<ShardConnection>? before = null)
        {
            await client.SignInAsGuestAsync();
            var join = await client.CallFnAsync("joinFrontier", PylonValue.Object(("frontier", frontier), ("size", 500)));
            var queue = new ConcurrentQueue<Action>();
            var o = new ShardConnectionOptions
            {
                BaseUrl = client.BaseUrl,
                SubscriberId = join["subscriberId"].AsString(),
                Ticket = join["ticket"].AsString(),
                Transport = ShardTransport.WebTransport,
                Dispatcher = PylonDispatcher.From(queue.Enqueue),
            };
            tweak(o);
            var shard = new ShardConnection(frontier, o);
            before?.Invoke(shard);
            return new Player(shard, queue);
        }

        /// <summary>Join, and wait for this player's entity: the one that appears after the join.</summary>
        static ulong Spawn(Player p)
        {
            ShardReplicationUpdate? first = null;
            p.Shard.Replication += u => first ??= u;
            p.Shard.Connect();
            p.Until("the first replication", () => first != null);
            var before = new HashSet<ulong>(p.Shard.Entities.Entities.Keys);
            Assert.True(p.Shard.Send("join") > 0);
            // Entity ids start at 0, so "none yet" is null.
            ulong? mine = null;
            p.Until("the player's entity", () =>
            {
                foreach (var id in p.Shard.Entities.Entities.Keys)
                {
                    // Ids only grow: this player, the newest, has the highest new one.
                    if (!before.Contains(id) && (mine == null || id > mine)) mine = id;
                }
                return mine.HasValue;
            });
            return mine!.Value;
        }

        static PylonValue MoveTo(double x, double y) =>
            PylonValue.Object(("move_to", PylonValue.Object(("x", x), ("y", y))));

        [LiveFact(Env)]
        public async Task AFrontierPlayerReplicatesOverWebTransport()
        {
            Assert.NotNull(WebTransportNative.Version());
            using var client = NewClient();
            using var p = await JoinFrontier(client, "frontier-wt", _ => { });
            var mine = Spawn(p);
            Assert.Equal(ShardTransport.WebTransport, p.Shard.Transport);

            var e = p.Shard.Entities.Get(mine)!;
            var start = (e.X, e.Y);
            var seq = p.Shard.Send(MoveTo(10, 10));
            Assert.True(seq > 0);
            // The move comes back in datagrams, and the ack with the tick that holds it.
            p.Until("the move", () => p.Shard.Ack >= seq && p.Shard.Entities.Get(mine) is { } now && (now.X, now.Y) != start);
            Assert.True(p.Shard.RttMs.HasValue);
            Assert.Equal(1, p.Opened);
            Assert.Empty(p.Errors);
        }

        [LiveFact(Env)]
        public async Task AutoPicksWebTransportAndFallsBackToAWebSocketWhenUdpIsBlocked()
        {
            using var client = NewClient();
            using (var auto = await JoinFrontier(client, "frontier-wt", o => o.Transport = ShardTransport.Auto))
            {
                Spawn(auto);
                Assert.Equal(ShardTransport.WebTransport, auto.Shard.Transport);
            }

            // Every UDP packet dropped: the session never opens, so Auto uses a WebSocket.
            using var proxy = new LossyUdpProxy(new IPEndPoint(IPAddress.Loopback, WtPort(client)), 1.0);
            using var blocked = await JoinFrontier(client, "frontier-wt", o =>
            {
                o.Transport = ShardTransport.Auto;
                o.WebTransportTimeout = TimeSpan.FromSeconds(1);
            }, s => ThroughProxy(s, proxy));
            Spawn(blocked);
            Assert.Equal(ShardTransport.WebSocket, blocked.Shard.Transport);
            Assert.True(proxy.Received > 0, "the plugin sent nothing to the proxy");
        }

        [LiveFact(Env)]
        public async Task AtTwoPercentLossNoFrameGapIsLongerThan500Ms()
        {
            using var client = NewClient();
            using var proxy = new LossyUdpProxy(new IPEndPoint(IPAddress.Loopback, WtPort(client)), 0.02);
            using var p = await JoinFrontier(client, "frontier-wt", _ => { }, s => ThroughProxy(s, proxy));
            var mine = Spawn(p);
            Assert.Equal(ShardTransport.WebTransport, p.Shard.Transport);

            var clock = Stopwatch.StartNew();
            var gaps = new List<double>();
            var long_ = new List<string>();
            double? last = null;
            var moves = 0;
            p.Shard.Replication += u =>
            {
                var now = clock.Elapsed.TotalMilliseconds;
                if (last.HasValue)
                {
                    gaps.Add(now - last.Value);
                    if (now - last.Value > 200)
                        long_.Add($"{now - last.Value:0} ms at {now / 1000:0.0} s to tick {u.Tick} (+{u.Summary.Spawned.Count} -{u.Summary.Despawned.Count} ~{u.Summary.Updated.Count} full={u.Summary.Full})");
                }
                last = now;
            };
            // Walk back and forth for 20 s, so every tick carries a change.
            var corners = new[] { (20.0, 20.0), (480.0, 480.0) };
            var lastMove = -1000.0;
            while (clock.Elapsed < TimeSpan.FromSeconds(20))
            {
                if (clock.Elapsed.TotalMilliseconds - lastMove > 1500)
                {
                    var (x, y) = corners[moves++ % 2];
                    p.Shard.Send(MoveTo(x, y));
                    lastMove = clock.Elapsed.TotalMilliseconds;
                }
                p.Pump();
                Thread.Sleep(1);
            }
            p.Pump();

            gaps.Sort();
            var max = gaps.Last();
            _out.WriteLine(
                $"{gaps.Count + 1} whole ticks; gap p50 {gaps[gaps.Count / 2]:0.0} ms, p99 {gaps[(int)(gaps.Count * 0.99)]:0.0} ms, max {max:0.0} ms; " +
                $"proxy dropped {proxy.Dropped} of {proxy.Received + proxy.Returned} packets (seed {proxy.Seed}); errors: {string.Join("; ", p.Errors)}; " +
                $"gaps over 200 ms: {string.Join(" | ", long_)}");
            Assert.True(proxy.Dropped > 0, "the proxy dropped nothing");
            // 20 Hz for 20 s: close to 400 whole ticks.
            Assert.True(gaps.Count > 300, $"{gaps.Count} gaps");
            Assert.True(max <= 500, $"a {max:0} ms gap between frames");
            Assert.Equal(1, p.Opened);
            Assert.Equal(ShardTransport.WebTransport, p.Shard.Transport);
            Assert.NotNull(p.Shard.Entities.Get(mine));
        }

        [LiveFact(Env)]
        public async Task AShardGameUsesWebTransportByDefault()
        {
            using var client = NewClient();
            await client.SignInAsGuestAsync();
            var join = await client.CallFnAsync("joinFrontier", PylonValue.Object(("frontier", "frontier-wt"), ("size", 500)));
            var queue = new ConcurrentQueue<Action>();
            using var game = new ShardGame<PylonValue>("frontier-wt", new ShardConnectionOptions
            {
                BaseUrl = client.BaseUrl,
                SubscriberId = join["subscriberId"].AsString(),
                Ticket = join["ticket"].AsString(),
                TickRate = 20,
                Dispatcher = PylonDispatcher.From(queue.Enqueue),
            }, PylonConverter.Value);
            var frames = 0;
            game.Replication += _ => frames++;
            game.Connect();
            var deadline = DateTime.UtcNow.AddSeconds(15);
            while (frames < 20 && DateTime.UtcNow < deadline)
            {
                while (queue.TryDequeue(out var a)) a();
                game.Frame();
                Thread.Sleep(5);
            }
            Assert.True(frames >= 20, $"{frames} frames");
            Assert.Equal(ShardTransport.WebTransport, game.Connection.Transport);
        }

        static int WtPort(PylonClient client)
        {
            using var http = new System.Net.Http.HttpClient();
            var body = http.GetStringAsync(new Uri(client.BaseUrl, "/_pylon/shard/webtransport")).Result;
            var (url, _) = ShardWebTransport.DecodeInfo(PylonValue.Parse(body));
            return new Uri(url).Port;
        }

        /// <summary>Send the session through the proxy: the endpoint info, with the proxy's address.</summary>
        static void ThroughProxy(ShardConnection shard, LossyUdpProxy proxy)
        {
            var real = shard.FetchInfo;
            shard.FetchInfo = async (url, ct) =>
            {
                var (status, body) = await real(url, ct);
                if (status != 200) return (status, body);
                var info = PylonValue.Parse(body);
                var hashes = info["certHashes"].ToJson();
                return (status, $"{{\"url\":\"https://127.0.0.1:{proxy.Port}/shard\",\"certHashes\":{hashes}}}");
            };
        }
    }

    /// <summary>
    /// A UDP relay to one server that drops a share of the packets in each
    /// direction at random: real loss for QUIC, on its stream and its datagrams.
    /// </summary>
    sealed class LossyUdpProxy : IDisposable
    {
        readonly UdpClient _front = new UdpClient(new IPEndPoint(IPAddress.Loopback, 0));
        readonly UdpClient _back = new UdpClient(new IPEndPoint(IPAddress.Loopback, 0));
        readonly IPEndPoint _server;
        readonly double _drop;
        readonly Random _random;
        readonly CancellationTokenSource _stop = new CancellationTokenSource();
        IPEndPoint? _client;
        long _received;
        long _returned;
        long _dropped;

        public LossyUdpProxy(IPEndPoint server, double drop, int? seed = null)
        {
            _server = server;
            _drop = drop;
            Seed = seed ?? Environment.TickCount;
            _random = new Random(Seed);
            _ = Task.Run(Forward);
            _ = Task.Run(Backward);
        }

        /// <summary>The seed of the drop pattern, to repeat a run.</summary>
        public readonly int Seed;

        public int Port => ((IPEndPoint)_front.Client.LocalEndPoint!).Port;
        /// <summary>Packets from the client.</summary>
        public long Received => Interlocked.Read(ref _received);
        /// <summary>Packets from the server.</summary>
        public long Returned => Interlocked.Read(ref _returned);
        public long Dropped => Interlocked.Read(ref _dropped);

        bool Drop()
        {
            lock (_random)
            {
                if (_random.NextDouble() >= _drop) return false;
            }
            Interlocked.Increment(ref _dropped);
            return true;
        }

        async Task Forward()
        {
            while (!_stop.IsCancellationRequested)
            {
                UdpReceiveResult r;
                try
                {
                    r = await _front.ReceiveAsync(_stop.Token);
                }
                catch (Exception)
                {
                    return;
                }
                Interlocked.Increment(ref _received);
                Volatile.Write(ref _client, r.RemoteEndPoint);
                if (!Drop()) await _back.SendAsync(r.Buffer, r.Buffer.Length, _server);
            }
        }

        async Task Backward()
        {
            while (!_stop.IsCancellationRequested)
            {
                UdpReceiveResult r;
                try
                {
                    r = await _back.ReceiveAsync(_stop.Token);
                }
                catch (Exception)
                {
                    return;
                }
                Interlocked.Increment(ref _returned);
                var client = Volatile.Read(ref _client);
                if (client != null && !Drop()) await _front.SendAsync(r.Buffer, r.Buffer.Length, client);
            }
        }

        public void Dispose()
        {
            _stop.Cancel();
            _front.Dispose();
            _back.Dispose();
        }
    }
}
