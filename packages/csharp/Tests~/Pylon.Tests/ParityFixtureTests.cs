using System.Collections.Generic;
using System.Linq;
using Pylon;
using Pylon.Realtime;
using Xunit;

namespace Pylon.Tests
{
    /// <summary>
    /// The fixtures the Rust encoders write (packages/realtime/src/*.fixtures.json).
    /// The TypeScript and Swift clients run the same files.
    /// </summary>
    public class ParityFixtureTests
    {
        static PylonValue TableJson(EntityTable table) =>
            PylonValue.Array(table.Entities.Values.OrderBy(e => e.Id).Select(e => PylonValue.Object(
                ("components", PylonValue.Object(e.Components.OrderBy(c => c.Key)
                    .Select(c => new KeyValuePair<string, PylonValue>(c.Key.ToString(), Hex.Of(c.Value))))),
                ("id", e.Id),
                ("q", PylonValue.Array(e.QX, e.QY, e.QZ)))));

        [Fact]
        public void ReplicationFramesApplyToTheSameTablesAsRust()
        {
            var fixtures = Fixtures.Load("replication.fixtures.json");
            var table = new EntityTable();
            var i = 0;
            foreach (var step in fixtures["frames"].Items)
            {
                table.Apply(Hex.Bytes(step["frame"].AsString()));
                Assert.True(step["table"].Equals(TableJson(table)), $"after frame {i}: {TableJson(table)}");
                foreach (var e in table.Entities.Values)
                    Assert.Equal(e.QX * step["precision"].AsDouble(), e.X, 9);
                i++;
            }
            Assert.Equal(11, i);
        }

        [Fact]
        public void DatagramsApplyAndSkipExactlyAsRust()
        {
            var fixtures = Fixtures.Load("replication.fixtures.json");
            var table = new EntityTable();
            var i = 0;
            foreach (var ev in fixtures["datagrams"].Items)
            {
                if (ev.Has("stream"))
                {
                    table.Apply(Hex.Bytes(ev["stream"].AsString()), ev["tick"].AsULong());
                }
                else
                {
                    var s = table.ApplyDatagram(Hex.Bytes(ev["datagram"].AsString()));
                    Assert.Equal(ev["frame"].AsULong(), s.Frame);
                    Assert.Equal(ev["tick"].AsULong(), s.Tick);
                    Assert.Equal(ev["ack"].AsULong(), s.Ack);
                    Assert.Equal(ev["sentStreamTick"].AsULong(), s.StreamTick);
                    Assert.Equal(ev["parts"].AsULong(), s.Parts);
                    Assert.Equal(ev["updated"].Items.Select(x => x.AsULong()).ToList(), s.Updated);
                    Assert.Equal(ev["skipped"].AsInt(), s.Skipped);
                    Assert.Equal(ev["streamTick"].AsULong(), table.StreamTick);
                }
                Assert.True(ev["table"].Equals(TableJson(table)), $"after event {i}: {TableJson(table)}");
                i++;
            }
            Assert.Equal(12, i);
        }

        [Fact]
        public void WireFramesParseAndDecodeAsRustBuiltThem()
        {
            var fixtures = Fixtures.Load("wire.fixtures.json");
            Assert.Equal(ShardWire.Version, fixtures["version"].AsInt());
            Assert.Equal(ShardWire.HeaderLength, fixtures["headerLength"].AsInt());
            var n = 0;
            foreach (var f in fixtures["frames"].Items)
            {
                var frame = ShardWire.Parse(Hex.Bytes(f["frame"].AsString()));
                Assert.Equal(f["kind"].AsInt(), frame.Kind);
                Assert.Equal(f["codec"].AsInt(), frame.Codec);
                Assert.Equal(f["tick"].AsULong(), frame.Tick);
                Assert.Equal(f["ack"].AsULong(), frame.Ack);
                var payload = ShardWire.DecodePayload(frame.Codec, frame.Payload);
                Assert.True(f["payload"].Equals(payload), $"frame {n}: {payload}");
                switch ((ShardFrameKind)frame.Kind)
                {
                    case ShardFrameKind.InputRejected:
                        var r = ShardWire.DecodeRejection(frame.Codec, frame.Payload);
                        Assert.Equal(f["payload"]["code"].AsString(), r.Code);
                        Assert.Equal(f["payload"]["message"].AsString(), r.Message);
                        var seq = f["payload"]["client_seq"];
                        Assert.Equal(seq.IsNull ? null : (ulong?)seq.AsULong(), r.ClientSeq);
                        break;
                    case ShardFrameKind.Transfer:
                        var t = ShardWire.DecodeTransfer(frame.Codec, frame.Payload);
                        Assert.Equal(f["payload"]["shard"].AsString(), t.Shard);
                        Assert.Equal(f["payload"]["ticket"].AsString(), t.Ticket);
                        break;
                }
                n++;
            }
            Assert.Equal(6, n);
        }

        [Fact]
        public void InputEnvelopesEncodeToTheBytesTheServerDecodes()
        {
            var fixtures = Fixtures.Load("wire.fixtures.json");
            foreach (var c in fixtures["inputs"].Items)
            {
                var seq = c["client_seq"].AsULong();
                ShardWire.EncodeInput((byte)ShardCodec.MessagePack, c["input"], seq, out var text, out var binary);
                Assert.Null(text);
                Assert.Equal(c["msgpack"].AsString(), Hex.Of(binary!));

                ShardWire.EncodeInput((byte)ShardCodec.Json, c["input"], seq, out text, out binary);
                Assert.Null(binary);
                Assert.True(PylonValue.Parse(c["json"].AsString()).Equals(PylonValue.Parse(text!)), text);

                // Before the first frame the codec is unknown: JSON goes.
                ShardWire.EncodeInput(null, c["input"], seq, out text, out binary);
                Assert.NotNull(text);
                Assert.Null(binary);
            }
        }
    }
}
