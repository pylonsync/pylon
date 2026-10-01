using System;
using System.Collections.Generic;
using System.Linq;
using Pylon.Realtime;
using Xunit;

namespace Pylon.Tests
{
    /// <summary>Ports of packages/realtime/src/{prediction,clock,interpolation}.test.ts.</summary>
    public class PredictorTests
    {
        static double Step(double x, double dx) => x + dx;

        [Fact]
        public void ReplaysTheInputsTheShardHasNotProcessed()
        {
            var p = new Predictor<double, double>(Step);
            p.Push(1, 1);
            p.Push(2, 2);
            p.Push(3, 4);
            Assert.Equal(7, p.Reconcile(1, 1));
            Assert.Equal(2, p.Count);
            Assert.Equal(5.5, p.Reconcile(1.5, 2));
            Assert.Equal(5.5, p.Reconcile(5.5, 3));
            Assert.Equal(0, p.Count);
        }

        [Fact]
        public void ForgetsRefusedAndUnsentInputs()
        {
            var p = new Predictor<double, double>(Step);
            p.Push(0, 100);
            p.Push(4, 1);
            p.Push(5, 2);
            p.Reject(4);
            p.Reject(null);
            Assert.Equal(2, p.Reconcile(0, 3));
        }

        [Fact]
        public void ResetAndBound()
        {
            var p = new Predictor<double, double>(Step, maxPending: 3);
            for (ulong seq = 1; seq <= 5; seq++) p.Push(seq, 1);
            Assert.Equal(3, p.Count);
            Assert.Equal(3, p.Reconcile(0, 0));
            p.Reset();
            Assert.Equal(10, p.Reconcile(10, 0));
        }
    }

    public class ShardClockTests
    {
        static Func<double> Rng(uint seed)
        {
            var s = seed;
            return () =>
            {
                s = unchecked(s * 1664525u + 1013904223u);
                return s / 4294967296.0;
            };
        }

        static void Feed(ShardClock clock, int from, int to, double latency, double jitter, uint seed = 1)
        {
            var r = Rng(seed);
            for (var tick = from; tick <= to; tick++) clock.Observe(tick, tick * 50 + latency + r() * jitter);
        }

        [Fact]
        public void NotReadyBeforeAFrame()
        {
            var clock = new ShardClock();
            Assert.False(clock.Ready);
            Assert.Equal(-1, clock.ServerTick(1000));
        }

        [Fact]
        public void AnchorsOnTheLeastDelayedFrame()
        {
            var clock = new ShardClock(20);
            Feed(clock, 1, 200, 40, 60);
            var est = clock.ServerTick(200 * 50 + 40 + 25);
            Assert.InRange(est, 200.3, 200.6);
        }

        [Fact]
        public void MeasuresTheTickLength()
        {
            var clock = new ShardClock();
            var r = Rng(7);
            for (var tick = 1; tick <= 128; tick++) clock.Observe(tick, tick * 33.3 + 20 + r() * 15);
            Assert.InRange(clock.TickMs, 32.8, 33.8);
        }

        [Fact]
        public void NeverGoesBackwardsWhenACorrectionArrives()
        {
            var clock = new ShardClock(20);
            Feed(clock, 1, 64, 100, 0);
            double t = 64 * 50 + 100;
            var prev = clock.ServerTick(t);
            for (var tick = 65; tick <= 200; tick++)
            {
                clock.Observe(tick, tick * 50 + 40);
                for (var step = 0; step < 3; step++)
                {
                    t += 50.0 / 3;
                    var now = clock.ServerTick(t);
                    Assert.True(now > prev);
                    prev = now;
                }
            }
            Assert.Equal(200, clock.ServerTick(200 * 50 + 40), 1);
        }

        [Fact]
        public void FollowsAShardWhoseTicksResumeLateAfterAStall()
        {
            var clock = new ShardClock(20);
            Feed(clock, 1, 40, 50, 0);
            for (var tick = 41; tick <= 43; tick++) clock.Observe(tick, tick * 50 + 50 + 2000);
            Assert.Equal(43, clock.ServerTick(43 * 50 + 50 + 2000), 1);
        }

        [Fact]
        public void OneLateFrameIsJitter()
        {
            var clock = new ShardClock(20);
            Feed(clock, 1, 40, 50, 0);
            clock.Observe(41, 41 * 50 + 50 + 2000);
            clock.Observe(42, 42 * 50 + 50);
            Assert.Equal(42, clock.ServerTick(42 * 50 + 50), 1);
        }

        [Fact]
        public void ABurstAfterANetworkStallCatchesUp()
        {
            var clock = new ShardClock(20);
            Feed(clock, 1, 40, 50, 0);
            double at = 40 * 50 + 50 + 2000;
            for (var tick = 41; tick <= 80; tick++) clock.Observe(tick, at);
            Assert.True(clock.ServerTick(at) > 78.9);
            Assert.Equal(110, clock.ServerTick(at + 1500), 1);
        }

        [Theory]
        [InlineData(20.0)]
        [InlineData(null)]
        public void ANetworkBurstNeitherMovesItBackNorThrowsItOff(double? tickRate)
        {
            var clock = new ShardClock(tickRate);
            var r = Rng(3);
            var prev = double.NegativeInfinity;
            double worst = 0;
            for (var tick = 1; tick <= 300; tick++)
            {
                double built = tick * 50;
                var at = tick >= 100 && tick <= 107 ? 107 * 50 + 50 : built + 50 + r() * 10;
                clock.Observe(tick, at);
                var est = clock.ServerTick(at);
                Assert.True(est >= prev);
                prev = est;
                if (tick > 120) worst = Math.Max(worst, Math.Abs(est - (at - 50) / 50));
            }
            Assert.True(worst < 1);
            Assert.InRange(clock.TickMs, 47, 53);
        }

        [Fact]
        public void AfterAStallItStepsBackNoFurtherThanTheNewestTick()
        {
            var clock = new ShardClock(20);
            Feed(clock, 1, 40, 50, 0);
            Assert.True(clock.ServerTick(40 * 50 + 50 + 1999) > 79);
            for (var tick = 41; tick <= 43; tick++)
            {
                double at = tick * 50 + 50 + 2000;
                clock.Observe(tick, at);
                Assert.True(clock.ServerTick(at) >= tick - 1);
            }
        }

        [Fact]
        public void TicksThatGoBackResetIt()
        {
            var clock = new ShardClock(20);
            Feed(clock, 1000, 1010, 30, 0);
            clock.Observe(3, 999_999);
            Assert.Equal(3, clock.LatestTick);
            Assert.Equal(3, clock.ServerTick(999_999), 5);
        }
    }

    public class EntityInterpolatorTests
    {
        sealed class Harness
        {
            public readonly EntityTable Table = new EntityTable();
            public readonly EntityInterpolator Interp;

            public Harness(EntityInterpolator? interp = null)
            {
                Interp = interp ?? new EntityInterpolator();
            }

            public void Frame(double tick, TestFrame f) => Interp.Record(Table, Table.Apply(f.Encode()), tick);

            public IReadOnlyDictionary<ulong, InterpolatedEntity> At(double renderTick)
            {
                Interp.Update(renderTick);
                return Interp.Entities;
            }
        }

        static TestFrame Full(params (ulong Id, double[] Pos, Dictionary<byte, byte[]?>? C)[] spawn)
        {
            var f = new TestFrame { Full = true };
            f.Spawn.AddRange(spawn);
            return f;
        }

        static TestFrame Move(ulong id, double[] from, double[] to, Dictionary<byte, byte[]?>? c = null)
        {
            var f = new TestFrame();
            f.Update.Add((id, from, to, c));
            return f;
        }

        static double[] P(double x = 0, double y = 0, double z = 0) => TestFrame.P(x, y, z);

        [Fact]
        public void DrawnTickIsTheTickAnUpdatePlacedEntitiesAtNeverGoingBack()
        {
            var h = new Harness();
            Assert.Equal(-1, h.Interp.DrawnTick);
            h.Frame(1, Full((1, P(), null)));
            h.Frame(9, Move(1, P(), P(8)));
            h.At(5.5);
            Assert.Equal(5.5, h.Interp.DrawnTick);
            // An earlier render tick is raised to what was drawn.
            h.At(3);
            Assert.Equal(5.5, h.Interp.DrawnTick);
            h.Interp.Clear();
            Assert.Equal(-1, h.Interp.DrawnTick);
        }

        [Fact]
        public void AnInputEnvelopeCarriesAViewTickOnlyWhenGiven()
        {
            ShardWire.EncodeInput(null, PylonValue.Object(("dx", 1)), 3, out var plain, out _);
            Assert.Equal("{\"input\":{\"dx\":1},\"client_seq\":3}", plain);
            ShardWire.EncodeInput(null, PylonValue.Object(("dx", 1)), 4, 17.25, out var stamped, out _);
            Assert.Equal(17.25, PylonValue.Parse(stamped!)["view_tick"].AsDouble());
            ShardWire.EncodeInput(null, PylonValue.Object(("dx", 1)), 5, double.NaN, out var nan, out _);
            Assert.True(PylonValue.Parse(nan!)["view_tick"].IsNull);
            ShardWire.EncodeInput((byte)ShardCodec.MessagePack, PylonValue.Object(("dx", 1)), 6, 2.5, out _, out var packed);
            Assert.Equal(2.5, MessagePack.Decode(packed!)["view_tick"].AsDouble());
        }

        [Fact]
        public void DrawsBetweenTheSamplesAroundTheRenderTick()
        {
            var h = new Harness();
            h.Frame(1, Full((1, P(), null)));
            h.Frame(2, Move(1, P(), P(10)));
            h.Frame(3, Move(1, P(10), P(20, -4)));
            Assert.Equal(5, h.At(1.5)[1].X, 5);
            var e = h.At(2.25)[1];
            Assert.Equal(12.5, e.X, 5);
            Assert.Equal(-1, e.Y, 5);
            Assert.Equal((2.0, 3.0, 0.25), (e.From.Tick, e.To.Tick, e.T));
            Assert.Equal(20, h.At(7)[1].X, 5);
        }

        [Fact]
        public void AppearsAtItsSpawnTickAndLeavesAtItsDespawnTick()
        {
            var h = new Harness();
            h.Frame(1, Full());
            var spawn = new TestFrame();
            spawn.Spawn.Add((3, P(1, 1, 1), null));
            h.Frame(5, spawn);
            var despawn = new TestFrame();
            despawn.Despawn.Add(3);
            h.Frame(8, despawn);
            Assert.False(h.At(4.9).ContainsKey(3));
            h.At(5);
            Assert.Equal(new ulong[] { 3 }, h.Interp.Entered);
            Assert.True(h.At(7.9).ContainsKey(3));
            h.At(8);
            Assert.Equal(new ulong[] { 3 }, h.Interp.Left);
            Assert.Empty(h.Interp.Entities);
        }

        [Fact]
        public void AReusedIdIsANewEntity()
        {
            var h = new Harness();
            h.Frame(1, Full((7, P(), new Dictionary<byte, byte[]?> { [1] = new byte[] { 1 } })));
            var f = new TestFrame();
            f.Despawn.Add(7);
            f.Spawn.Add((7, P(50), null));
            h.Frame(3, f);
            var old = h.At(2.5)[7];
            Assert.Equal(0, old.X, 5);
            Assert.Equal(new byte[] { 1 }, old.Components[1]);
            var fresh = h.At(3)[7];
            Assert.Equal(new ulong[] { 7 }, h.Interp.Left);
            Assert.Equal(new ulong[] { 7 }, h.Interp.Entered);
            Assert.NotSame(old, fresh);
            Assert.Equal(50, fresh.X, 5);
            Assert.Empty(fresh.Components);
        }

        [Fact]
        public void AQuietEntityStandsStillUntilTheTickBeforeItsNextMove()
        {
            var h = new Harness();
            h.Frame(1, Full((1, P(), null)));
            h.Frame(10, Move(1, P(), P(10)));
            Assert.Equal(0, h.At(5)[1].X, 5);
            Assert.Equal(5, h.At(9.5)[1].X, 5);

            var spread = new Harness(new EntityInterpolator(holdWhenQuiet: false));
            spread.Frame(1, Full((1, P(), null)));
            spread.Frame(11, Move(1, P(), P(10)));
            Assert.Equal(5, spread.At(6)[1].X, 5);
        }

        [Fact]
        public void ALongMoveJumps()
        {
            var h = new Harness(new EntityInterpolator(snapDistance: 5));
            h.Frame(1, Full((1, P(), null)));
            h.Frame(2, Move(1, P(), P(100)));
            h.Frame(3, Move(1, P(100), P(102)));
            Assert.Equal(0, h.At(1.9)[1].X, 5);
            Assert.Equal(100, h.At(2)[1].X, 5);
            Assert.Equal(101, h.At(2.5)[1].X, 5);
        }

        [Fact]
        public void ComponentsChangeAtTheirTick()
        {
            var h = new Harness();
            h.Frame(1, Full((1, P(), new Dictionary<byte, byte[]?> { [1] = new byte[] { 100 } })));
            h.Frame(3, Move(1, P(), P(), new Dictionary<byte, byte[]?> { [1] = new byte[] { 40 }, [2] = new byte[] { 9 } }));
            h.Frame(4, Move(1, P(), P(), new Dictionary<byte, byte[]?> { [2] = null }));
            Assert.Equal(new byte[] { 100 }, h.At(2.9)[1].Components[1]);
            var e = h.At(3.5)[1];
            Assert.Equal(new byte[] { 40 }, e.Components[1]);
            Assert.False(e.To.Components.ContainsKey(2));
            Assert.False(h.At(4)[1].Components.ContainsKey(2));
        }

        [Fact]
        public void AFullFrameEndsEntitiesItLacks()
        {
            var h = new Harness();
            h.Frame(1, Full((1, P(), null), (2, P(), null)));
            h.At(1);
            h.Frame(5, Full((2, P(4), null)));
            Assert.True(h.At(4.5).ContainsKey(1));
            var e = h.At(5)[2];
            Assert.Equal(new ulong[] { 1 }, h.Interp.Left);
            Assert.Empty(h.Interp.Entered);
            Assert.Equal(4, e.X, 5);
        }

        [Fact]
        public void TicksThatGoBackStartOver()
        {
            var h = new Harness();
            h.Frame(100, Full((1, P(5), null)));
            h.At(100);
            h.Frame(1, Full((9, P(), null)));
            Assert.Equal(new ulong[] { 9 }, h.At(1).Keys.ToArray());
            Assert.Equal(new ulong[] { 1 }, h.Interp.Left);
        }

        [Fact]
        public void KeepsABoundedHistory()
        {
            var h = new Harness(new EntityInterpolator(maxSamples: 4));
            h.Frame(0, Full((1, P(), null)));
            for (var t = 1; t <= 50; t++) h.Frame(t, Move(1, P(t - 1), P(t)));
            Assert.Equal(47, h.At(10)[1].X, 5);
            Assert.Equal(49.5, h.At(49.5)[1].X, 5);
        }

        [Fact]
        public void ARenderTickNeverGoesBack()
        {
            var h = new Harness();
            h.Frame(10, Full((5, P(), null)));
            h.Frame(12, Move(5, P(), P(2)));
            h.At(11);
            Assert.Equal(new ulong[] { 5 }, h.Interp.Entered);
            h.At(9);
            Assert.Empty(h.Interp.Left);
            Assert.Equal(0, h.Interp.Entities[5].X, 5);
            h.At(11.5);
            Assert.Empty(h.Interp.Entered);
            Assert.Equal(1, h.Interp.Entities[5].X, 5);
        }

        [Fact]
        public void EndedEntitiesDoNotPileUp()
        {
            var h = new Harness();
            h.Frame(0, Full());
            var tick = 1;
            for (ulong id = 1; id <= 5000; id++)
            {
                var s = new TestFrame();
                s.Spawn.Add((id, P(), null));
                h.Frame(tick++, s);
                var d = new TestFrame();
                d.Despawn.Add(id);
                h.Frame(tick++, d);
            }
            Assert.Empty(h.At(tick + 1));
        }
    }

    public class ReplicationTests
    {
        static byte[] Head(params byte[] rest) => new byte[] { 1, 0, 0x0a, 0xd7, 0x23, 0x3c }.Concat(rest).ToArray();

        [Fact]
        public void RefusesBadFrames()
        {
            var t = new EntityTable();
            Assert.Throws<ReplicationException>(() => t.Apply(new byte[0]));
            Assert.Contains("version 2", Assert.Throws<ReplicationException>(() => t.Apply(new byte[] { 2, 0, 0, 0, 0x80, 0x3f })).Message);
            Assert.Contains("bad precision", Assert.Throws<ReplicationException>(() => t.Apply(new byte[] { 1, 0, 0, 0, 0, 0 })).Message);
            Assert.Contains("count too large", Assert.Throws<ReplicationException>(() => t.Apply(Head(0xff, 0xff, 0xff, 0xff, 0x0f))).Message);
            Assert.Contains("unknown entity 7", Assert.Throws<ReplicationException>(() => t.Apply(Head(0, 0, 1, 7, 0))).Message);
            Assert.Contains("do not ascend", Assert.Throws<ReplicationException>(() => t.Apply(Head(2, 5, 0, 0, 0))).Message);
            Assert.Contains("trailing", Assert.Throws<ReplicationException>(() => t.Apply(Head(0, 0, 0, 9))).Message);
            Assert.Contains("past 64 bits", Assert.Throws<ReplicationException>(() =>
                t.Apply(Head(1, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f, 0, 0))).Message);
            Assert.Throws<ReplicationException>(() => t.ApplyDatagram(new byte[] { 1 }));
        }

        [Fact]
        public void HoldsIdsAndPositionsBeyondTwoToThe53()
        {
            // Spawn id 2^63 at x = 2^60 (zigzag doubles it).
            var f = new List<byte> { 1, 1 };
            f.AddRange(BitConverter.GetBytes(1f));
            f.Add(0); // despawns
            f.Add(1); // spawns
            f.AddRange(new byte[] { 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01 });
            f.AddRange(new byte[] { 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x20 });
            f.Add(0);
            f.Add(0);
            f.Add(0); // components
            f.Add(0); // updates
            var t = new EntityTable();
            t.Apply(f.ToArray());
            var e = t.Get(1UL << 63)!;
            Assert.Equal(1L << 60, e.QX);
        }
    }
}
