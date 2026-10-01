using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Linq;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using Pylon;
using Pylon.Realtime;
using Xunit;

namespace Pylon.Tests
{
    /// <summary>
    /// ShardConnection over WebTransport, against a fake session the test
    /// plays the server of. A port of packages/realtime/src/connection-webtransport.test.ts.
    /// </summary>
    public class WebTransportConnectionTests : IDisposable
    {
        static readonly byte[] Hash = Enumerable.Repeat((byte)7, 32).ToArray();

        readonly FakeWebTransport _wt = new FakeWebTransport();
        readonly ConcurrentQueue<Action> _queue = new ConcurrentQueue<Action>();
        readonly List<string> _fetched = new List<string>();
        readonly List<string> _aborted = new List<string>();
        readonly List<ShardConnection> _clients = new List<ShardConnection>();
        int _infoStatus = 200;
        bool _infoHangs;

        // Rust-encoded: a full frame with entities 1, 2, 3, then a datagram for
        // frame 2 (tick 3, ack 30) and one for frame 1 (tick 2, ack 20).
        static readonly PylonValue Datagrams = Fixtures.Load("replication.fixtures.json")["datagrams"];
        static byte[] FullFrame => Hex.Bytes(Datagrams[0]["stream"].AsStringOr("")!);
        static byte[] Datagram2 => Hex.Bytes(Datagrams[1]["datagram"].AsStringOr("")!);
        static byte[] Datagram1 => Hex.Bytes(Datagrams[2]["datagram"].AsStringOr("")!);

        public void Dispose()
        {
            foreach (var c in _clients) c.Dispose();
        }

        ShardConnection Make(string shard = "field", Action<ShardConnectionOptions>? tweak = null, Uri? baseUrl = null)
        {
            var o = new ShardConnectionOptions
            {
                BaseUrl = baseUrl ?? new Uri("http://h"),
                SubscriberId = "p1",
                Transport = ShardTransport.WebTransport,
                Dispatcher = PylonDispatcher.From(_queue.Enqueue),
                ReconnectBaseDelay = TimeSpan.FromMilliseconds(1),
            };
            tweak?.Invoke(o);
            var c = new ShardConnection(shard, o)
            {
                WebTransports = _wt,
                FetchInfo = async (url, ct) =>
                {
                    lock (_fetched) _fetched.Add(url.ToString());
                    if (_infoHangs)
                    {
                        try
                        {
                            await Task.Delay(Timeout.Infinite, ct);
                        }
                        catch (OperationCanceledException)
                        {
                            lock (_aborted) _aborted.Add(url.ToString());
                            throw;
                        }
                    }
                    if (_infoStatus != 200) return (_infoStatus, "{}");
                    return (200, $"{{\"url\":\"https://wt.example:443/shard\",\"certHashes\":[\"{Convert.ToBase64String(Hash)}\"]}}");
                },
            };
            _clients.Add(c);
            return c;
        }

        void Pump()
        {
            while (_queue.TryDequeue(out var a)) a();
        }

        void Until(string what, Func<bool> done, int ms = 5000)
        {
            var deadline = DateTime.UtcNow.AddMilliseconds(ms);
            while (true)
            {
                Pump();
                if (done()) return;
                if (DateTime.UtcNow > deadline) throw new TimeoutException($"timed out waiting for {what}");
                Thread.Sleep(2);
            }
        }

        /// <summary>Let the poll loop and the dispatcher run for a while.</summary>
        void Settle(int ms = 30)
        {
            var deadline = DateTime.UtcNow.AddMilliseconds(ms);
            while (DateTime.UtcNow < deadline)
            {
                Pump();
                Thread.Sleep(2);
            }
            Pump();
        }

        static byte[] Frame(byte kind, byte codec, ulong tick, ulong ack, byte[] payload) =>
            ShardWire.Frame(kind, codec, tick, ack, payload);

        static byte[] Json(string s) => Encoding.UTF8.GetBytes(s);

        static IEnumerable<byte> Varint(ulong v)
        {
            while (v >= 0x80)
            {
                yield return (byte)((v & 0x7f) | 0x80);
                v >>= 7;
            }
            yield return (byte)v;
        }

        static byte[] Precision => BitConverter.GetBytes(0.01f);

        /// <summary>
        /// A datagram for the fixture's full frame (precision 0.01, entities
        /// 1-3 spawned at tick 1): each update sets x.
        /// </summary>
        static byte[] Dg(ulong frame, ulong tick, ulong ack, ulong parts, (ulong Id, ulong X)[] xs, ulong streamTick = 1)
        {
            var out_ = new List<byte> { 2 };
            out_.AddRange(Varint(frame));
            out_.AddRange(Varint(tick));
            out_.AddRange(Varint(ack));
            out_.AddRange(Varint(streamTick));
            out_.AddRange(Precision);
            out_.AddRange(Varint(parts));
            out_.AddRange(Varint((ulong)xs.Length));
            ulong? last = null;
            foreach (var (id, x) in xs)
            {
                out_.AddRange(Varint(last == null ? id : id - last.Value));
                out_.AddRange(new byte[] { 1, 0, 1 });
                out_.AddRange(Varint(x * 2));
                last = id;
            }
            return out_.ToArray();
        }

        static long Qx(ShardConnection c, ulong id) => c.Entities.Get(id)?.QX ?? -1;

        (ShardConnection Client, FakeSession Wt, List<ulong> Acks) OpenWithBaseline(Action<ShardConnectionOptions>? tweak = null)
        {
            var acks = new List<ulong>();
            var client = Make(tweak: o =>
            {
                o.Ticket = "t1";
                tweak?.Invoke(o);
            });
            client.Replication += u =>
            {
                if (!u.Summary.Full) acks.Add(u.Ack);
            };
            client.Connect();
            Until("the session", () => client.Connected);
            var wt = _wt.Last;
            wt.SendFrames(Frame(3, 4, 1, 0, FullFrame));
            Until("the full frame", () => client.Entities.Count == 3);
            return (client, wt, acks);
        }

        [Fact]
        public void ASessionSendsTheHelloAppliesStreamFramesAndDatagramsAcksThemAndSendsInputs()
        {
            var updates = new List<ShardReplicationUpdate>();
            var summaries = new List<(bool Full, ulong[] Spawned, ulong[] Updated, ulong[] Despawned)>();
            var client = Make(tweak: o => o.Ticket = "t1", baseUrl: new Uri("http://app.example"));
            client.Replication += u =>
            {
                updates.Add(u);
                summaries.Add((u.Summary.Full, u.Summary.Spawned.ToArray(), u.Summary.Updated.ToArray(), u.Summary.Despawned.ToArray()));
            };
            client.Connect();
            Until("the session", () => client.Connected);
            Assert.Equal(ShardTransport.WebTransport, client.Transport);
            Assert.Equal(new[] { "http://app.example/_pylon/shard/webtransport" }, _fetched);

            var wt = _wt.Last;
            Assert.Equal("https://wt.example:443/shard", wt.Url);
            Assert.Single(wt.Hashes);
            Assert.Equal(Hash, wt.Hashes[0]);
            Until("the hello", () => wt.StreamIn.Count == 1);
            var hello = PylonValue.Parse(Encoding.UTF8.GetString(wt.StreamIn[0]));
            Assert.Equal("field", hello["shard"].AsStringOr(null));
            Assert.Equal("p1", hello["sid"].AsStringOr(null));
            Assert.Equal("t1", hello["ticket"].AsStringOr(null));
            Assert.True(hello["token"].IsNull);

            // The baseline on the stream, split across chunks.
            wt.SendFrames(new[] { Frame(3, 4, 1, 0, FullFrame) }, 3);
            Until("the full frame", () => client.Entities.Count == 3);
            Assert.True(summaries[0].Full);
            Assert.Equal(1UL, updates[0].Tick);

            // An update datagram: applied, then acked with the frames applied.
            wt.SendDatagram(Datagram2);
            Until("the ack", () => wt.DatagramsIn.Count == 1);
            Assert.Equal(new byte[] { 1, 1, 2, 1 }, wt.DatagramsIn[0]);
            Assert.False(summaries[1].Full);
            Assert.Empty(summaries[1].Spawned);
            Assert.Empty(summaries[1].Despawned);
            Assert.Equal(Datagrams[1]["updated"].Items.Select(v => v.AsULong()).ToArray(), summaries[1].Updated);
            Assert.Equal(Datagrams[1]["tick"].AsULong(), updates[1].Tick);
            Assert.Equal(3UL, client.Tick);
            Assert.Equal(30UL, client.Ack);
            Assert.Equal(175, Qx(client, 1));

            // An older tick's datagram, late: dropped unacked, as if lost.
            wt.SendDatagram(Datagram1);
            wt.SendFrames(Frame(1, 0, 4, 30, Json("{\"n\":1}")));
            Until("the snapshot", () => client.Tick == 4);
            Assert.Single(wt.DatagramsIn);
            Assert.Equal(2, updates.Count);

            // Inputs go on the stream after a type byte: JSON until the shard's
            // codec is known (the snapshot said JSON).
            Assert.Equal(1UL, client.Send(PylonValue.Object(("move", 1))));
            Until("the input", () => wt.StreamIn.Count == 2);
            Assert.Equal(2, wt.StreamIn[1][0]);
            var input = PylonValue.Parse(Encoding.UTF8.GetString(wt.StreamIn[1], 1, wt.StreamIn[1].Length - 1));
            Assert.Equal(1L, input["input"]["move"].AsLong());
            Assert.Equal(1UL, input["client_seq"].AsULong());

            client.Dispose();
            Until("the close", () => wt.ClientClose != null);
            Assert.Equal((0u, ""), wt.ClientClose!.Value);
            Assert.False(client.Connected);
        }

        [Fact]
        public void ARefusalOfFixedCredentialsStopsTheClient()
        {
            var states = new List<(ShardConnectionState State, string? Reason)>();
            var client = Make(tweak: o => o.Ticket = "expired");
            client.StateChanged += (s, r) => states.Add((s, r));
            client.Connect();
            Until("the session", () => client.Connected);
            _wt.Sessions[0].ServerClose(1, "unauthorized: ticket expired");
            Until("the refusal", () => client.State == ShardConnectionState.Failed);
            Pump();
            Assert.Contains("refused the connection (unauthorized: ticket expired)", states.Last().Reason);
            Settle();
            Assert.Single(_wt.Sessions);
        }

        [Fact]
        public void ASessionCloseWithCodeOneButAnotherReasonReconnects()
        {
            var client = Make(tweak: o => o.Ticket = "t1");
            client.Connect();
            Until("the session", () => client.Connected);
            _wt.Sessions[0].ServerClose(1, "shard not found");
            Until("the second session", () => _wt.Sessions.Count == 2 && client.Connected);
        }

        [Fact]
        public void ATransferReconnectsWithTheTicketItCarries()
        {
            var client = Make("west", o => o.Ticket = "t-west");
            client.Connect();
            Until("the session", () => client.Connected);
            var first = _wt.Sessions[0];
            first.SendFrames(Frame(4, 0, 9, 0, Json("{\"shard\":\"east\",\"ticket\":\"t-east\"}")));
            Until("the transfer", () => client.ShardId == "east");
            first.ServerClose(0, "moved to shard east");
            Until("the second session", () => _wt.Sessions.Count == 2 && client.Connected);
            var second = _wt.Sessions[1];
            Until("the hello", () => second.StreamIn.Count == 1);
            var hello = PylonValue.Parse(Encoding.UTF8.GetString(second.StreamIn[0]));
            Assert.Equal("east", hello["shard"].AsStringOr(null));
            Assert.Equal("t-east", hello["ticket"].AsStringOr(null));
        }

        /// <summary>A WebSocket server that holds each connection open, and counts them.</summary>
        static FakeShardServer HoldingServer()
        {
            var server = new FakeShardServer();
            server.OnConnection = async (_, ws) =>
            {
                try
                {
                    while (true) await FakeShardServer.Receive(ws);
                }
                catch (Exception)
                {
                    // The client closed.
                }
            };
            return server;
        }

        static int Count(FakeShardServer s)
        {
            lock (s.Requests) return s.Requests.Count;
        }

        [Fact]
        public void AutoUsesAWebSocketForGoodWhenTheAppDoesNotServeWebTransport()
        {
            using var server = new FakeShardServer();
            // Each connection is dropped at once: the client reconnects.
            server.OnConnection = async (_, ws) =>
                await ws.CloseOutputAsync(System.Net.WebSockets.WebSocketCloseStatus.NormalClosure, "", default);
            _infoStatus = 404;
            var opened = 0;
            var client = Make(tweak: o => o.Transport = ShardTransport.Auto, baseUrl: server.Base);
            client.Opened += () => opened++;
            client.Connect();
            Until("the socket", () => opened >= 1);
            Assert.Empty(_wt.Sessions);
            Assert.Equal(ShardTransport.WebSocket, client.Transport);
            Until("the reconnect", () => Count(server) >= 2);
            Assert.Single(_fetched);
        }

        [Fact]
        public void AutoFallsBackToAWebSocketWhenTheSessionFails()
        {
            using var server = HoldingServer();
            _wt.NextReady = "fail";
            var client = Make(tweak: o => o.Transport = ShardTransport.Auto, baseUrl: server.Base);
            client.Connect();
            Until("the socket", () => client.Connected);
            Assert.Equal(ShardTransport.WebSocket, client.Transport);
            Assert.Single(_wt.Sessions);
        }

        [Fact]
        public void AutoFallsBackToAWebSocketWhenTheSessionDoesNotOpenInTime()
        {
            using var server = HoldingServer();
            _wt.NextReady = "hang";
            var client = Make(tweak: o =>
            {
                o.Transport = ShardTransport.Auto;
                o.WebTransportTimeout = TimeSpan.FromMilliseconds(20);
            }, baseUrl: server.Base);
            client.Connect();
            Until("the socket", () => client.Connected);
            Assert.Equal(ShardTransport.WebSocket, client.Transport);
            Assert.NotNull(_wt.Sessions[0].ClientClose);
            Assert.True(_wt.Sessions[0].Disposed);
        }

        [Fact]
        public void AutoGoesStraightToAWebSocketWithoutThePlugin()
        {
            using var server = HoldingServer();
            _wt.Available = false;
            var client = Make(tweak: o => o.Transport = ShardTransport.Auto, baseUrl: server.Base);
            client.Connect();
            Until("the socket", () => client.Connected);
            Assert.Empty(_fetched);
            Assert.Empty(_wt.Sessions);
        }

        [Fact]
        public void AForcedWebTransportStopsWithAnErrorWhenTheAppDoesNotServeIt()
        {
            _infoStatus = 404;
            var errors = new List<string>();
            using var server = HoldingServer();
            var client = Make(baseUrl: server.Base);
            client.Error += e => errors.Add(e.Message);
            client.Connect();
            Until("the error", () => errors.Count == 1);
            Assert.Contains("the app does not serve WebTransport", errors[0]);
            Until("the stop", () => client.State == ShardConnectionState.Failed);
            Settle();
            Assert.Single(_fetched);
            Assert.Equal(0, Count(server));
        }

        [Fact]
        public void AForcedWebTransportRetriesASessionThatFails()
        {
            _wt.NextReady = "fail";
            var errors = new List<string>();
            var client = Make(tweak: o => o.Ticket = "t1");
            client.Error += e => errors.Add(e.Message);
            client.Connect();
            Until("the second session", () => _wt.Sessions.Count == 2 && client.Connected);
            Assert.Contains("the session did not open", errors[0]);
        }

        [Fact]
        public void AReconnectStartsTheTableOverSoUpdatesApply()
        {
            var fulls = 0;
            var client = Make(tweak: o => o.Ticket = "t1");
            client.Replication += u =>
            {
                if (u.Summary.Full) fulls++;
            };
            client.Connect();
            Until("the session", () => client.Connected);
            var first = _wt.Sessions[0];
            first.SendFrames(Frame(3, 4, 1, 0, FullFrame));
            Until("the full frame", () => fulls == 1);
            first.SendDatagram(Datagram2);
            Until("the update", () => Qx(client, 1) == 175);

            // The server drops the client; the new subscription starts from scratch
            // and sends the same frames (its ticks and datagram numbers restart).
            first.ServerClose(3, "client too slow");
            Until("the second session", () => _wt.Sessions.Count == 2 && client.Connected);
            var second = _wt.Sessions[1];
            second.SendFrames(Frame(3, 4, 1, 0, FullFrame));
            Until("the full frame", () => fulls == 2);
            Assert.Equal(100, Qx(client, 1));
            second.SendDatagram(Datagram2);
            Until("the update", () => Qx(client, 1) == 175);
        }

        [Fact]
        public void AutoFallsBackToAWebSocketWhenTheEndpointRequestStalls()
        {
            using var server = HoldingServer();
            _infoHangs = true;
            var client = Make(tweak: o =>
            {
                o.Transport = ShardTransport.Auto;
                o.WebTransportTimeout = TimeSpan.FromMilliseconds(20);
            }, baseUrl: server.Base);
            client.Connect();
            Until("the socket", () => client.Connected);
            Assert.Single(_aborted);
            Assert.Empty(_wt.Sessions);
        }

        [Fact]
        public void DisposingTheClientStopsTheEndpointRequest()
        {
            using var server = HoldingServer();
            _infoHangs = true;
            var client = Make(baseUrl: server.Base);
            client.Connect();
            Until("the request", () => _fetched.Count == 1);
            client.Dispose();
            Until("the abort", () => _aborted.Count == 1);
            Settle();
            Assert.Equal(0, Count(server));
            Assert.Empty(_wt.Sessions);
        }

        [Fact]
        public void ATicksDatagramsAndItsAckApplyTogetherOnceTheTickIsWhole()
        {
            var (client, wt, acks) = OpenWithBaseline();

            // Tick 2 in two parts: the first alone changes nothing yet.
            wt.SendDatagram(Dg(1, 2, 5, 2, new[] { (1UL, 500UL) }));
            Settle();
            Assert.Equal(100, Qx(client, 1));
            Assert.Equal(0UL, client.Ack);
            Assert.Empty(wt.DatagramsIn);

            wt.SendDatagram(Dg(2, 2, 5, 2, new[] { (2UL, 600UL) }));
            Until("tick 2", () => client.Ack == 5);
            Assert.Equal(500, Qx(client, 1));
            Assert.Equal(600, Qx(client, 2));
            Assert.Equal(new[] { 5UL }, acks);
            Until("the ack", () => wt.DatagramsIn.Count == 1);
            Assert.Equal(new byte[] { 1, 2, 1, 1, 2, 1 }, wt.DatagramsIn[0]);

            // Tick 3 loses a part; tick 4 is whole, so tick 3's part applies first.
            wt.SendDatagram(Dg(3, 3, 6, 2, new[] { (1UL, 700UL) }));
            wt.SendDatagram(Dg(5, 4, 7, 1, new[] { (3UL, 800UL) }));
            Until("tick 4", () => client.Ack == 7);
            Assert.Equal(700, Qx(client, 1));
            Assert.Equal(800, Qx(client, 3));
            Assert.Equal(new[] { 5UL, 7UL }, acks);

            // The lost part turns up late: dropped, not acked.
            Until("the acks", () => wt.DatagramsIn.Count == 2);
            wt.SendDatagram(Dg(4, 3, 6, 2, new[] { (2UL, 900UL) }));
            Settle();
            Assert.Equal(600, Qx(client, 2));
            Assert.Equal(2, wt.DatagramsIn.Count);
        }

        [Fact]
        public void ATickWaitsForTheStreamFramesSentByThen()
        {
            var (client, wt, _) = OpenWithBaseline();
            wt.SendDatagram(Dg(1, 5, 9, 1, new[] { (1UL, 300UL) }, streamTick: 5));
            Settle();
            Assert.Equal(0UL, client.Ack);
            // Tick 5's stream frame: nothing spawned or despawned, one empty update list.
            var empty = new List<byte> { 1, 0 };
            empty.AddRange(Precision);
            empty.AddRange(new byte[] { 0, 0, 0 });
            wt.SendFrames(Frame(3, 4, 5, 9, empty.ToArray()));
            Until("tick 5", () => client.Ack == 9);
            Assert.Equal(300, Qx(client, 1));
        }

        [Fact]
        public void AClosingFrameRefusesFixedCredentials()
        {
            var states = new List<(ShardConnectionState State, string? Reason)>();
            var client = Make(tweak: o => o.Ticket = "expired");
            client.StateChanged += (s, r) => states.Add((s, r));
            client.Connect();
            Until("the session", () => client.Connected);
            var wt = _wt.Sessions[0];
            wt.SendFrames(Frame(6, 0, 0, 0, Json("{\"code\":1,\"reason\":\"unauthorized: ticket expired\"}")));
            Until("the refusal", () => client.State == ShardConnectionState.Failed);
            Pump();
            Assert.NotNull(wt.ClientClose);
            Assert.Contains("refused the connection (unauthorized: ticket expired)", states.Last().Reason);
            Settle();
            Assert.Single(_wt.Sessions);
        }

        [Fact]
        public void AClosingFrameNamesTheCloseCodeAndReason()
        {
            var closes = new List<ShardCloseInfo>();
            var client = Make(tweak: o => o.Ticket = "t1");
            client.Closed += c => closes.Add(c);
            client.Connect();
            Until("the session", () => client.Connected);
            _wt.Sessions[0].SendFrames(Frame(6, 0, 0, 0, Json("{\"code\":3,\"reason\":\"client too slow\"}")));
            Until("the close", () => closes.Count == 1);
            Assert.Equal(3, closes[0].Code);
            Assert.Equal("client too slow", closes[0].Reason);
            Assert.True(closes[0].WebTransport);
            Until("the reconnect", () => _wt.Sessions.Count == 2 && client.Connected);
        }

        [Fact]
        public void AStreamFrameWaitsForItsTicksDatagramsAndTheHandlersGetBothWithTheTicksAck()
        {
            var (client, wt, _) = OpenWithBaseline();
            var seen = new List<(ulong[] Spawned, ulong[] Updated, ulong Ack)>();
            client.Replication += u =>
            {
                if (!u.Summary.Full) seen.Add((u.Summary.Spawned.ToArray(), u.Summary.Updated.ToArray(), u.Ack));
            };
            // Tick 2's stream frame: entity 4 spawns at x = 50 (q 5000).
            var spawn = new List<byte> { 1, 0 };
            spawn.AddRange(Precision);
            spawn.AddRange(new byte[] { 0, 1, 4 });
            spawn.AddRange(Varint(10000));
            spawn.AddRange(new byte[] { 0, 0, 0, 0 });
            wt.SendFrames(Frame(3, 4, 2, 3, spawn.ToArray()));
            Settle();
            Assert.Null(client.Entities.Get(4));
            Assert.Empty(seen);

            wt.SendDatagram(Dg(1, 2, 3, 1, new[] { (1UL, 150UL) }, streamTick: 2));
            Until("tick 2", () => seen.Count == 1);
            Assert.Equal(5000, Qx(client, 4));
            Assert.Equal(new[] { 4UL }, seen[0].Spawned);
            Assert.Equal(new[] { 1UL }, seen[0].Updated);
            Assert.Equal(3UL, seen[0].Ack);
            // Acked with the stream tick the table had then.
            Until("the ack", () => wt.DatagramsIn.Count == 1);
            Assert.Equal(new byte[] { 1, 1, 1, 2 }, wt.DatagramsIn[0]);
        }

        [Fact]
        public void ASessionWhoseDatagramsStopArrivingCloses()
        {
            var errors = new List<string>();
            var (client, wt, _) = OpenWithBaseline();
            client.Error += e => errors.Add(e.Message);
            // Every tick misses a part, so none is ever whole.
            for (ulong tick = 2; tick <= 103; tick++) wt.SendDatagram(Dg(tick, tick, 0, 2, new[] { (1UL, tick) }));
            Until("the close", () => wt.ClientClose != null);
            Pump();
            Assert.Contains("WebTransport to shard field: datagrams stopped arriving", errors);
        }

        [Fact]
        public void AutoUsesWebSocketsAfterAStall()
        {
            using var server = HoldingServer();
            var errors = new List<string>();
            var (client, wt, _) = OpenWithBaseline(o =>
            {
                o.Transport = ShardTransport.Auto;
                o.BaseUrl = server.Base;
            });
            client.Error += e => errors.Add(e.Message);
            for (ulong tick = 2; tick <= 103; tick++) wt.SendDatagram(Dg(tick, tick, 0, 2, new[] { (1UL, tick) }));
            Until("the socket", () => client.Connected && client.Transport == ShardTransport.WebSocket);
            Assert.Single(_wt.Sessions);
        }

        [Fact]
        public void ASessionWithNoWholeTickFor5SecondsClosesEvenWhenNothingArrives()
        {
            double clock = 1000;
            var errors = new List<string>();
            var (client, wt, _) = OpenWithBaseline(o => o.Now = () => Volatile.Read(ref clock));
            client.Error += e => errors.Add(e.Message);
            // Silence: no stream frame, no datagram. The watchdog runs each second.
            Volatile.Write(ref clock, 5000);
            Settle(1100);
            Assert.Null(wt.ClientClose);
            Volatile.Write(ref clock, 7000);
            Until("the close", () => wt.ClientClose != null, 3000);
            Pump();
            Assert.Contains("WebTransport to shard field: datagrams stopped arriving", errors);
        }

        [Fact]
        public void ASnapshotShardIsNotTakenForAStall()
        {
            double clock = 1000;
            var client = Make(tweak: o =>
            {
                o.Ticket = "t1";
                o.Now = () => Volatile.Read(ref clock);
            });
            client.Connect();
            Until("the session", () => client.Connected);
            var wt = _wt.Last;
            wt.SendFrames(Frame(1, 0, 1, 0, Json("{\"n\":1}")));
            Until("the snapshot", () => client.Tick == 1);
            Volatile.Write(ref clock, 11000);
            Settle(1100);
            Assert.Null(wt.ClientClose);
            Assert.True(client.Connected);
        }

        [Fact]
        public void AShardGameUsesAutoUnlessToldOtherwise()
        {
            using var server = HoldingServer();
            ShardGame<PylonValue> Game(ShardTransport? transport)
            {
                var game = new ShardGame<PylonValue>("field", new ShardConnectionOptions
                {
                    BaseUrl = server.Base,
                    SubscriberId = "p1",
                    Transport = transport,
                    Dispatcher = PylonDispatcher.From(_queue.Enqueue),
                }, PylonConverter.Value);
                game.Connection.WebTransports = _wt;
                game.Connection.FetchInfo = (url, ct) =>
                {
                    lock (_fetched) _fetched.Add(url.ToString());
                    return Task.FromResult((404, "{}"));
                };
                return game;
            }

            using (var auto = Game(null))
            {
                auto.Connect();
                Until("the socket", () => auto.Connected);
                Assert.Single(_fetched);
                Assert.Equal(ShardTransport.WebSocket, auto.Connection.Transport);
            }
            using (var ws = Game(ShardTransport.WebSocket))
            {
                ws.Connect();
                Until("the socket", () => ws.Connected);
                Assert.Single(_fetched);
            }
            // A plain connection with no transport set uses a WebSocket.
            var plain = Make(tweak: o => o.Transport = null, baseUrl: server.Base);
            plain.Connect();
            Until("the socket", () => plain.Connected);
            Assert.Single(_fetched);
        }

        [Fact]
        public void AForcedWebTransportThatFailsToOpenStopsWithoutAutoReconnect()
        {
            _wt.NextReady = "fail";
            var states = new List<ShardConnectionState>();
            var client = Make(tweak: o =>
            {
                o.Ticket = "t1";
                o.AutoReconnect = false;
            });
            client.StateChanged += (s, _) => states.Add(s);
            client.Connect();
            Until("the stop", () => states.Count >= 2 && states.Last() == ShardConnectionState.Disconnected);
            Settle(100);
            Assert.Single(_wt.Sessions);
            Assert.Equal(ShardConnectionState.Disconnected, client.State);
        }

        [Fact]
        public void SendReturnsZeroWhenThePeerStopsTakingTheStream()
        {
            var errors = new List<string>();
            var (client, wt, _) = OpenWithBaseline(o => o.MaxFrameBytes = 4096);
            client.Error += e => errors.Add(e.Message);
            Assert.Equal(1UL, client.Send(PylonValue.Object(("move", 1))));
            // The peer holds nearly the whole limit: the next input does not fit.
            Interlocked.Exchange(ref wt.Held, 4090);
            Assert.Equal(0UL, client.Send(PylonValue.Object(("move", 2))));
            Pump();
            Assert.Contains(errors, e => e.Contains("4096 bytes of inputs are waiting to go out"));
            // Once the peer takes them, inputs go again, numbered on from the last one sent.
            Interlocked.Exchange(ref wt.Held, 0);
            Assert.Equal(2UL, client.Send(PylonValue.Object(("move", 3))));
        }

        [Fact]
        public void AStreamFrameOverMaxFrameBytesEndsTheSessionBeforeItIsBuffered()
        {
            var closes = new List<ShardCloseInfo>();
            var client = Make(tweak: o =>
            {
                o.Ticket = "t1";
                o.MaxFrameBytes = 1000;
                o.AutoReconnect = false;
            });
            client.Closed += c => closes.Add(c);
            client.Connect();
            Until("the session", () => client.Connected);
            var wt = _wt.Last;
            // Only the prefix of a 2000-byte frame: refused before its payload arrives.
            wt.SendRaw(new byte[] { 0, 0, 0x07, 0xd0, 1, 2, 3 });
            Until("the close", () => closes.Count == 1);
            Assert.Contains("over 1000 bytes", closes[0].Reason);
            Assert.NotNull(wt.ClientClose);
        }

        [Fact]
        public void StreamFramesChecksTheLimitAsSoonAsThePrefixArrives()
        {
            var frames = new StreamFrames(10);
            Assert.Single(frames.Push(new byte[] { 0, 0, 0, 2, 7, 8 }, 6));
            var e = Assert.Throws<PylonException>(() => frames.Push(new byte[] { 0, 0, 0, 11 }, 4));
            Assert.Contains("over 10 bytes", e.Message);
            // A prefix split across chunks is checked once it is whole.
            var split = new StreamFrames(10);
            Assert.Empty(split.Push(new byte[] { 0, 0 }, 2));
            Assert.Throws<PylonException>(() => split.Push(new byte[] { 0, 11 }, 2));
        }

        [Fact]
        public void TheClientNeedsPluginAbi11OrLaterWithin1()
        {
            Assert.True(WebTransportNative.Compatible("1.1.0"));
            Assert.True(WebTransportNative.Compatible("1.4.2"));
            Assert.False(WebTransportNative.Compatible("1.0.0"));
            Assert.False(WebTransportNative.Compatible("2.1.0"));
            Assert.False(WebTransportNative.Compatible("0.22.12"));
            Assert.False(WebTransportNative.Compatible("1"));
            Assert.False(WebTransportNative.Compatible(null));
        }

        [Fact]
        public void AckBatchesFitTheSessionsMaxDatagramSize()
        {
            var acks = Enumerable.Range(1, 2000).Select(i => ((ulong)i * 1000, (ulong)i * 1000)).ToList();
            var batches = ShardWebTransport.DatagramAckBatches(acks, 100).ToList();
            Assert.All(batches, b => Assert.True(b.Length <= 100, $"{b.Length} bytes"));
            // Every ack goes out once, in order.
            var decoded = batches.SelectMany(DecodeAcks).ToList();
            Assert.Equal(acks, decoded);
        }

        static IEnumerable<(ulong, ulong)> DecodeAcks(byte[] message)
        {
            Assert.Equal(1, message[0]);
            var pos = 1;
            ulong Read()
            {
                ulong v = 0;
                var shift = 0;
                while (true)
                {
                    var b = message[pos++];
                    v |= (ulong)(b & 0x7f) << shift;
                    if (b < 0x80) return v;
                    shift += 7;
                }
            }
            var count = Read();
            var list = new List<(ulong, ulong)>();
            for (ulong i = 0; i < count; i++) list.Add((Read(), Read()));
            Assert.Equal(message.Length, pos);
            return list;
        }
    }

    /// <summary>Opens <see cref="FakeSession"/>s.</summary>
    sealed class FakeWebTransport : IWebTransportFactory
    {
        readonly List<FakeSession> _sessions = new List<FakeSession>();
        public volatile bool Available = true;
        /// <summary>"open", "fail" (the handshake fails), or "hang" (it never finishes). Applies to the next session.</summary>
        public volatile string NextReady = "open";

        bool IWebTransportFactory.Available => Available;

        public List<FakeSession> Sessions
        {
            get
            {
                lock (_sessions) return _sessions.ToList();
            }
        }

        public FakeSession Last => Sessions.Last();

        public IWebTransportSession? Connect(string url, byte[][] certHashes)
        {
            var s = new FakeSession(url, certHashes, NextReady);
            NextReady = "open";
            lock (_sessions) _sessions.Add(s);
            return s;
        }
    }

    /// <summary>A WebTransport session the test plays the server of.</summary>
    sealed class FakeSession : IWebTransportSession
    {
        readonly object _gate = new object();
        readonly Queue<byte> _toClient = new Queue<byte>();
        readonly Queue<byte[]> _datagramsToClient = new Queue<byte[]>();
        readonly List<byte> _streamBytes = new List<byte>();
        readonly List<byte[]> _streamIn = new List<byte[]>();
        readonly List<byte[]> _datagramsIn = new List<byte[]>();
        int _state;
        (uint, string)? _serverClose;
        (uint, string)? _clientClose;

        public readonly string Url;
        public readonly byte[][] Hashes;
        public volatile bool Disposed;

        public FakeSession(string url, byte[][] hashes, string mode)
        {
            Url = url;
            Hashes = hashes;
            _state = mode == "open" ? WebTransportNative.StateOpen
                : mode == "fail" ? WebTransportNative.StateFailed
                : WebTransportNative.StateConnecting;
        }

        /// <summary>Messages the client wrote on its stream, split at the length prefixes.</summary>
        public List<byte[]> StreamIn
        {
            get
            {
                lock (_gate) return _streamIn.ToList();
            }
        }

        /// <summary>Datagrams the client sent.</summary>
        public List<byte[]> DatagramsIn
        {
            get
            {
                lock (_gate) return _datagramsIn.ToList();
            }
        }

        public (uint Code, string Reason)? ClientClose
        {
            get
            {
                lock (_gate) return _clientClose;
            }
        }

        /// <summary>Send frames on the stream, each length-prefixed, in chunks of <paramref name="split"/> bytes.</summary>
        public void SendFrames(byte[][] frames, int split)
        {
            var all = new List<byte>();
            foreach (var f in frames) all.AddRange(ShardWebTransport.LengthPrefixed(f));
            // The chunks arrive one by one, as a QUIC stream delivers them.
            for (var i = 0; i < all.Count; i += split)
            {
                lock (_gate)
                {
                    foreach (var b in all.Skip(i).Take(split)) _toClient.Enqueue(b);
                }
            }
        }

        public void SendFrames(byte[] frame) => SendFrames(new[] { frame }, 5);

        /// <summary>Raw stream bytes, no length prefix added.</summary>
        public void SendRaw(byte[] bytes)
        {
            lock (_gate)
            {
                foreach (var b in bytes) _toClient.Enqueue(b);
            }
        }

        public void SendDatagram(byte[] d)
        {
            lock (_gate) _datagramsToClient.Enqueue(d);
        }

        public void ServerClose(uint code, string reason)
        {
            lock (_gate)
            {
                _serverClose = (code, reason);
                _state = WebTransportNative.StateClosed;
            }
        }

        public int State
        {
            get
            {
                lock (_gate) return _state;
            }
        }

        public int MaxDatagramSize => 1200;

        /// <summary>Stream bytes the test says the peer has not taken.</summary>
        public long Held;

        public long StreamQueued => Interlocked.Read(ref Held);

        bool IWebTransportSession.SendDatagram(byte[] data)
        {
            lock (_gate) _datagramsIn.Add(data);
            return true;
        }

        public int RecvDatagram(byte[] buf)
        {
            lock (_gate)
            {
                if (_datagramsToClient.Count == 0) return WebTransportNative.ErrNone;
                var d = _datagramsToClient.Dequeue();
                Buffer.BlockCopy(d, 0, buf, 0, d.Length);
                return d.Length;
            }
        }

        public bool StreamWrite(byte[] data)
        {
            lock (_gate)
            {
                if (_state != WebTransportNative.StateOpen) return false;
                _streamBytes.AddRange(data);
                while (_streamBytes.Count >= 4)
                {
                    var len = (_streamBytes[0] << 24) | (_streamBytes[1] << 16) | (_streamBytes[2] << 8) | _streamBytes[3];
                    if (_streamBytes.Count < 4 + len) break;
                    _streamIn.Add(_streamBytes.Skip(4).Take(len).ToArray());
                    _streamBytes.RemoveRange(0, 4 + len);
                }
                return true;
            }
        }

        public long StreamRead(byte[] buf)
        {
            lock (_gate)
            {
                if (_toClient.Count == 0) return _state == WebTransportNative.StateOpen ? 0 : WebTransportNative.ErrStreamEnded;
                var n = 0;
                while (n < buf.Length && _toClient.Count > 0) buf[n++] = _toClient.Dequeue();
                return n;
            }
        }

        public void Close(uint code, string reason)
        {
            lock (_gate)
            {
                _clientClose ??= (code, reason);
                if (_state == WebTransportNative.StateOpen || _state == WebTransportNative.StateConnecting)
                    _state = WebTransportNative.StateClosed;
            }
        }

        public (uint Code, string Reason)? CloseInfo
        {
            get
            {
                lock (_gate) return _serverClose;
            }
        }

        public string Error => _state == WebTransportNative.StateFailed ? "connect: QUIC handshake failed" : "";

        public void Dispose() => Disposed = true;
    }
}
