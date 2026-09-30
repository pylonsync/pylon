using System;
using Pylon;
using Xunit;

namespace Pylon.Tests
{
    public class ValueTests
    {
        [Theory]
        [InlineData("null")]
        [InlineData("true")]
        [InlineData("false")]
        [InlineData("0")]
        [InlineData("-9223372036854775808")]
        [InlineData("18446744073709551615")]
        [InlineData("1.5")]
        [InlineData("\"a\\\"b\\\\c\\n\\u0001\"")]
        [InlineData("[1,[2,[]],{}]")]
        [InlineData("{\"b\":1,\"a\":{\"c\":[null]}}")]
        public void JsonRoundTrips(string json)
        {
            var v = PylonValue.Parse(json);
            Assert.Equal(json, v.ToJson());
            Assert.Equal(v, PylonValue.Parse(v.ToJson()));
        }

        [Fact]
        public void JsonKeepsNumberKindsAndUnicode()
        {
            Assert.Equal(PylonValueKind.Int, PylonValue.Parse("42").Kind);
            Assert.Equal(PylonValueKind.UInt, PylonValue.Parse("18446744073709551615").Kind);
            Assert.Equal(PylonValueKind.Float, PylonValue.Parse("42.0").Kind);
            Assert.Equal(PylonValueKind.Float, PylonValue.Parse("1e3").Kind);
            Assert.Equal(PylonValueKind.Float, PylonValue.Parse("-18446744073709551616").Kind);
            Assert.Equal("é中😀", PylonValue.Parse("\"\\u00e9\\u4e2d\\ud83d\\ude00\"").AsString());
            Assert.Equal("5.0", PylonValue.From(5.0).ToJson());
            Assert.Equal("\"\\u2028\"", PylonValue.From("\u2028").ToJson());
        }

        [Theory]
        [InlineData("")]
        [InlineData("tru")]
        [InlineData("01")]
        [InlineData("1.")]
        [InlineData("-")]
        [InlineData("[1,]")]
        [InlineData("{\"a\"}")]
        [InlineData("{a:1}")]
        [InlineData("\"\u0001\"")]
        [InlineData("\"\\x\"")]
        [InlineData("1 2")]
        [InlineData("1e999")]
        public void JsonRejectsInvalidText(string json)
        {
            var e = Assert.Throws<PylonException>(() => PylonValue.Parse(json));
            Assert.Equal(PylonErrorKind.Decoding, e.Kind);
        }

        [Fact]
        public void JsonRefusesDeepNesting()
        {
            Assert.Throws<PylonException>(() => PylonValue.Parse(new string('[', 600) + new string(']', 600)));
        }

        [Fact]
        public void MissingPathsReadAsNullAndWrongTypesThrow()
        {
            var v = PylonValue.Parse("{\"players\":[{\"x\":1.5}]}");
            Assert.Equal(1.5, v["players"][0]["x"].AsDouble());
            Assert.True(v["players"][3]["x"].IsNull);
            Assert.True(v["nope"]["deeper"][0].IsNull);
            Assert.Equal("fallback", v["nope"].AsStringOr("fallback"));
            Assert.Throws<PylonException>(() => v["players"].AsString());
            Assert.Throws<PylonException>(() => PylonValue.From(1.5).AsLong());
            Assert.Equal(2L, PylonValue.From(2.0).AsLong());
            Assert.Throws<PylonException>(() => PylonValue.From(-1).AsULong());
        }

        [Fact]
        public void ObjectsKeepOrderAndTheLastValueOfARepeatedKey()
        {
            var v = PylonValue.Parse("{\"z\":1,\"a\":2,\"z\":3}");
            Assert.Equal("{\"z\":3,\"a\":2}", v.ToJson());
            var big = PylonValue.Object(("k0", 0), ("k1", 1), ("k2", 2), ("k3", 3), ("k4", 4), ("k5", 5),
                ("k6", 6), ("k7", 7), ("k8", 8), ("k9", 9));
            Assert.Equal(9, big["k9"].AsInt());
            Assert.False(big.Has("k10"));
        }

        [Fact]
        public void EqualityComparesNumbersByValueAndObjectsWithoutOrder()
        {
            Assert.Equal(PylonValue.From(1), PylonValue.From(1.0));
            Assert.NotEqual(PylonValue.From(1), PylonValue.From("1"));
            Assert.Equal(PylonValue.Parse("{\"a\":1,\"b\":[true]}"), PylonValue.Parse("{\"b\":[true],\"a\":1.0}"));
            Assert.NotEqual(PylonValue.Parse("{\"a\":1}"), PylonValue.Parse("{\"a\":1,\"b\":2}"));
        }

        [Fact]
        public void BytesAreCopiedAndWriteAsBase64()
        {
            var raw = new byte[] { 1, 2, 3 };
            var v = PylonValue.From(raw);
            raw[0] = 9;
            Assert.Equal(new byte[] { 1, 2, 3 }, v.AsBytes());
            Assert.Equal("\"AQID\"", v.ToJson());
            Assert.Throws<PylonException>(() => PylonValue.From(double.NaN).ToJson());
        }

        [Fact]
        public void MessagePackUsesTheSmallestIntegerEncodings()
        {
            string Enc(PylonValue v) => Hex.Of(MessagePack.Encode(v));
            Assert.Equal("7f", Enc(127));
            Assert.Equal("cc80", Enc(128));
            Assert.Equal("cd0100", Enc(256));
            Assert.Equal("ce00010000", Enc(65536));
            Assert.Equal("cf0000000100000000", Enc(4294967296L));
            Assert.Equal("ff", Enc(-1));
            Assert.Equal("e0", Enc(-32));
            Assert.Equal("d0df", Enc(-33));
            Assert.Equal("d1ff7f", Enc(-129));
            Assert.Equal("d2ffff7fff", Enc(-32769));
            Assert.Equal("d3ffffffff7fffffff", Enc(-2147483649L));
            Assert.Equal("cfffffffffffffffff", Enc(ulong.MaxValue));
            Assert.Equal("cb3ff8000000000000", Enc(1.5));
            Assert.Equal("c0", Enc(PylonValue.Null));
            Assert.Equal("c3", Enc(true));
            Assert.Equal("a161", Enc("a"));
            Assert.Equal("c40201ff", Enc(new byte[] { 1, 255 }));
            Assert.Equal("81a1619101", Enc(PylonValue.Object(("a", PylonValue.Array(1)))));
        }

        [Fact]
        public void MessagePackRoundTripsEveryKind()
        {
            var v = PylonValue.Object(
                ("s", new string('x', 40)),
                ("long", new string('y', 300)),
                ("arr", PylonValue.Array(new PylonValue[20])),
                ("n", PylonValue.Array(0, -1, 70000, long.MinValue, ulong.MaxValue, 2.25, true, false)),
                ("b", new byte[300]),
                ("nested", PylonValue.Object(("deep", PylonValue.Object(("x", PylonValue.Null))))));
            var back = MessagePack.Decode(MessagePack.Encode(v));
            Assert.Equal(v, back);
            Assert.Equal(PylonValueKind.UInt, back["n"][4].Kind);
        }

        [Fact]
        public void MessagePackReadsFloat32AndIntegerKeys()
        {
            // {1: float32 1.5}
            var v = MessagePack.Decode(Hex.Bytes("8101ca3fc00000"));
            Assert.Equal(1.5, v["1"].AsDouble());
        }

        [Theory]
        [InlineData("")]
        [InlineData("a3616263ff")]
        [InlineData("dc0005")]
        [InlineData("c4ff")]
        [InlineData("c7010100")]
        [InlineData("81c001")]
        [InlineData("a2c328")]
        public void MessagePackRejectsBadBytes(string hex)
        {
            Assert.Throws<PylonException>(() => MessagePack.Decode(Hex.Bytes(hex)));
        }

        [Fact]
        public void ConvertersWrapFunctions()
        {
            var c = PylonConverter.Create<(int X, int Y)>(
                p => PylonValue.Object(("x", p.X), ("y", p.Y)),
                v => (v["x"].AsInt(), v["y"].AsInt()));
            Assert.Equal((3, 4), c.FromValue(c.ToValue((3, 4))));
        }
    }
}
