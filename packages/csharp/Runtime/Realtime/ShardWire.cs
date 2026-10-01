#nullable enable
using System;
using System.Text;

namespace Pylon.Realtime
{
    /// <summary>Frame kinds of wire protocol version 2.</summary>
    public enum ShardFrameKind : byte
    {
        Snapshot = 1,
        InputRejected = 2,
        /// <summary>An entity replication frame: apply it to an <see cref="EntityTable"/>.</summary>
        Replication = 3,
        /// <summary>The subscriber moved to another shard. The last frame on the connection.</summary>
        Transfer = 4,
        /// <summary>A datagram frame (wire version 3 only).</summary>
        Datagram = 5,
        /// <summary>The server is about to close the session (WebTransport only).</summary>
        Closing = 6,
    }

    /// <summary>Payload codecs a frame header names.</summary>
    public enum ShardCodec : byte
    {
        Json = 0,
        MessagePack = 1,
        Bincode = 2,
        /// <summary>A game's own codec.</summary>
        Custom = 3,
        /// <summary>The replication frame format (frame kind 3).</summary>
        Replication = 4,
    }

    /// <summary>One server frame: the 18-byte header and the payload.</summary>
    public readonly struct ShardFrame
    {
        public readonly byte Kind;
        public readonly byte Codec;
        public readonly ulong Tick;
        /// <summary>The highest <c>client_seq</c> the shard processed for this subscriber (0 = none).</summary>
        public readonly ulong Ack;
        public readonly ArraySegment<byte> Payload;

        public ShardFrame(byte kind, byte codec, ulong tick, ulong ack, ArraySegment<byte> payload)
        {
            Kind = kind;
            Codec = codec;
            Tick = tick;
            Ack = ack;
            Payload = payload;
        }
    }

    /// <summary>An input the shard refused.</summary>
    public sealed class ShardInputRejection
    {
        public ulong? ClientSeq { get; }
        /// <summary>
        /// <c>unauthorized</c>, <c>rate_limited</c>, <c>queue_full</c>,
        /// <c>invalid</c>, <c>stopped</c>, <c>transferring</c>, or <c>apply_failed</c>.
        /// </summary>
        public string Code { get; }
        public string Message { get; }

        public ShardInputRejection(ulong? clientSeq, string code, string message)
        {
            ClientSeq = clientSeq;
            Code = code;
            Message = message;
        }
    }

    /// <summary>Where the subscriber went: connect to <see cref="Shard"/> with <see cref="Ticket"/>.</summary>
    public sealed class ShardTransferNotice
    {
        public string Shard { get; }
        public string Ticket { get; }

        public ShardTransferNotice(string shard, string ticket)
        {
            Shard = shard;
            Ticket = ticket;
        }
    }

    /// <summary>
    /// The shard wire protocol, version 2 (<c>pylon_realtime::wire</c> in
    /// Rust). A client asks for it with <c>?v=2</c>. Each server message is
    /// a binary frame:
    /// <code>
    /// 0   1  frame kind (ShardFrameKind)
    /// 1   1  codec (ShardCodec)
    /// 2   8  tick, u64 big-endian
    /// 10  8  ack: highest client_seq processed for this subscriber (0 = none)
    /// 18  .. payload in the codec
    /// </code>
    /// Inputs go up as <c>{ input, client_seq }</c>: JSON in a text frame,
    /// or MessagePack in a binary frame for a MessagePack shard.
    /// </summary>
    public static class ShardWire
    {
        public const int Version = 2;
        public const int HeaderLength = 18;

        public static ShardFrame Parse(byte[] data) => Parse(new ArraySegment<byte>(data));

        public static ShardFrame Parse(ArraySegment<byte> data)
        {
            if (data.Count < HeaderLength) throw PylonException.Decoding($"shard frame too short ({data.Count} bytes)");
            var b = data.Array!;
            var o = data.Offset;
            return new ShardFrame(
                b[o],
                b[o + 1],
                ReadU64(b, o + 2),
                ReadU64(b, o + 10),
                new ArraySegment<byte>(b, o + HeaderLength, data.Count - HeaderLength));
        }

        /// <summary>Build a frame. For tests and tools; the server builds real ones.</summary>
        public static byte[] Frame(byte kind, byte codec, ulong tick, ulong ack, byte[] payload)
        {
            var out_ = new byte[HeaderLength + payload.Length];
            out_[0] = kind;
            out_[1] = codec;
            WriteU64(out_, 2, tick);
            WriteU64(out_, 10, ack);
            Buffer.BlockCopy(payload, 0, out_, HeaderLength, payload.Length);
            return out_;
        }

        static ulong ReadU64(byte[] b, int at)
        {
            ulong v = 0;
            for (var i = 0; i < 8; i++) v = (v << 8) | b[at + i];
            return v;
        }

        static void WriteU64(byte[] b, int at, ulong v)
        {
            for (var i = 7; i >= 0; i--)
            {
                b[at + i] = (byte)v;
                v >>= 8;
            }
        }

        /// <summary>Decode a JSON or MessagePack payload. Other codecs throw.</summary>
        public static PylonValue DecodePayload(byte codec, ArraySegment<byte> payload)
        {
            switch ((ShardCodec)codec)
            {
                case ShardCodec.Json:
                    return PylonValue.Parse(Encoding.UTF8.GetString(payload.Array!, payload.Offset, payload.Count));
                case ShardCodec.MessagePack:
                    return MessagePack.Decode(payload.Array!, payload.Offset, payload.Count);
                default:
                    throw PylonException.Decoding(
                        $"shard codec {codec} needs a custom decoder (use the raw payload of the event)");
            }
        }

        public static ShardInputRejection DecodeRejection(byte codec, ArraySegment<byte> payload)
        {
            var v = DecodePayload(codec, payload);
            var seq = v["client_seq"];
            return new ShardInputRejection(
                seq.IsNumber ? (ulong?)seq.AsULong() : null,
                v["code"].AsStringOr(null) ?? "invalid",
                v["message"].AsStringOr(null) ?? "");
        }

        public static ShardTransferNotice DecodeTransfer(byte codec, ArraySegment<byte> payload)
        {
            var v = DecodePayload(codec, payload);
            var shard = v["shard"].AsStringOr(null);
            var ticket = v["ticket"].AsStringOr(null);
            if (shard == null || ticket == null) throw PylonException.Decoding("transfer frame without shard and ticket");
            return new ShardTransferNotice(shard, ticket);
        }

        /// <summary>
        /// The envelope <c>{ input, client_seq }</c>. <paramref name="text"/>
        /// is the JSON for a text frame, or null when <paramref name="binary"/>
        /// holds MessagePack for a binary frame (a MessagePack shard). Before
        /// the first frame the codec is unknown and JSON goes, which every
        /// shard accepts.
        /// </summary>
        public static void EncodeInput(byte? codec, PylonValue input, ulong clientSeq, out string? text, out byte[]? binary)
        {
            var envelope = PylonValue.Object(("input", input ?? PylonValue.Null), ("client_seq", clientSeq));
            if (codec == (byte)ShardCodec.MessagePack)
            {
                text = null;
                binary = MessagePack.Encode(envelope);
            }
            else
            {
                text = envelope.ToJson();
                binary = null;
            }
        }
    }

    /// <summary>
    /// WebTransport framing (wire version 3 in Rust): the same v2 frames
    /// travel on one bidirectional stream, each after a 4-byte big-endian
    /// length, and entity updates travel as datagrams.
    /// </summary>
    public static class ShardWebTransport
    {
        /// <summary>Type bytes of the client's stream messages.</summary>
        public const byte MessageInput = 0;
        public const byte MessageAcks = 1;
        public const byte MessageJsonInput = 2;

        /// <summary>Acks one message may carry (the server refuses more).</summary>
        public const int MaxAcksPerMessage = 512;

        /// <summary>Session close codes the server uses.</summary>
        public const uint CloseNormal = 0;
        /// <summary>Refused: bad credentials, an unknown shard.</summary>
        public const uint ClosePolicy = 1;
        public const uint CloseProtocol = 2;
        /// <summary>Try again: the client was too slow, or the server was busy.</summary>
        public const uint CloseAgain = 3;

        /// <summary>A stream message: a 4-byte big-endian length, then the bytes.</summary>
        public static byte[] LengthPrefixed(byte[] bytes)
        {
            var out_ = new byte[4 + bytes.Length];
            out_[0] = (byte)(bytes.Length >> 24);
            out_[1] = (byte)(bytes.Length >> 16);
            out_[2] = (byte)(bytes.Length >> 8);
            out_[3] = (byte)bytes.Length;
            Buffer.BlockCopy(bytes, 0, out_, 4, bytes.Length);
            return out_;
        }

        /// <summary>The first stream message: who connects to which shard.</summary>
        public static byte[] Hello(string shard, string sid, string? ticket, string? token)
        {
            var fields = new System.Collections.Generic.List<(string, PylonValue)> { ("shard", shard), ("sid", sid) };
            if (!string.IsNullOrEmpty(ticket)) fields.Add(("ticket", ticket));
            if (!string.IsNullOrEmpty(token)) fields.Add(("token", token));
            return LengthPrefixed(Encoding.UTF8.GetBytes(PylonValue.Object(fields.ToArray()).ToJson()));
        }

        /// <summary>An input as a stream message: the type byte (JSON or the shard's codec), then the envelope.</summary>
        public static byte[] Input(byte[] envelope, bool json)
        {
            var msg = new byte[1 + envelope.Length];
            msg[0] = json ? MessageJsonInput : MessageInput;
            Buffer.BlockCopy(envelope, 0, msg, 1, envelope.Length);
            return LengthPrefixed(msg);
        }

        static void Varint(System.Collections.Generic.List<byte> out_, ulong v)
        {
            while (v >= 0x80)
            {
                out_.Add((byte)(v | 0x80));
                v >>= 7;
            }
            out_.Add((byte)v);
        }

        static int VarintLength(ulong v)
        {
            var n = 1;
            while (v >= 0x80)
            {
                v >>= 7;
                n++;
            }
            return n;
        }

        /// <summary>
        /// Datagram acks: the type byte, a varint count, then per ack the
        /// datagram's frame number and the table's stream tick when it applied, both varints.
        /// </summary>
        public static byte[] DatagramAcks(System.Collections.Generic.IReadOnlyList<(ulong Frame, ulong Applied)> acks)
        {
            var out_ = new System.Collections.Generic.List<byte> { MessageAcks };
            Varint(out_, (ulong)acks.Count);
            foreach (var (frame, applied) in acks)
            {
                Varint(out_, frame);
                Varint(out_, applied);
            }
            return out_.ToArray();
        }

        /// <summary><see cref="DatagramAcks"/> split into messages of at most <paramref name="maxBytes"/> bytes and <see cref="MaxAcksPerMessage"/> acks.</summary>
        public static System.Collections.Generic.List<byte[]> DatagramAckBatches(
            System.Collections.Generic.IReadOnlyList<(ulong Frame, ulong Applied)> acks, int maxBytes)
        {
            var out_ = new System.Collections.Generic.List<byte[]>();
            var start = 0;
            // The type byte plus the count, which is below 2^14 (two varint bytes).
            var size = 3;
            for (var i = 0; i < acks.Count; i++)
            {
                var n = VarintLength(acks[i].Frame) + VarintLength(acks[i].Applied);
                if (i > start && (size + n > maxBytes || i - start == MaxAcksPerMessage))
                {
                    out_.Add(DatagramAcks(Slice(acks, start, i)));
                    start = i;
                    size = 3;
                }
                size += n;
            }
            if (start < acks.Count) out_.Add(DatagramAcks(Slice(acks, start, acks.Count)));
            return out_;
        }

        static System.Collections.Generic.List<(ulong, ulong)> Slice(
            System.Collections.Generic.IReadOnlyList<(ulong Frame, ulong Applied)> acks, int from, int to)
        {
            var l = new System.Collections.Generic.List<(ulong, ulong)>(to - from);
            for (var i = from; i < to; i++) l.Add(acks[i]);
            return l;
        }

        /// <summary>The body of <c>GET /_pylon/shard/webtransport</c>: the URL and the certificate hashes to pin.</summary>
        public static (string Url, byte[][] CertHashes) DecodeInfo(PylonValue body)
        {
            var url = body["url"].AsStringOr(null) ?? throw PylonException.Decoding("WebTransport info without a url");
            var list = new System.Collections.Generic.List<byte[]>();
            foreach (var h in body["certHashes"].Items)
            {
                var b64 = h.AsStringOr(null) ?? throw PylonException.Decoding("a WebTransport certificate hash is not a string");
                byte[] bytes;
                try
                {
                    bytes = Convert.FromBase64String(b64);
                }
                catch (FormatException)
                {
                    throw PylonException.Decoding("a WebTransport certificate hash is not base64");
                }
                if (bytes.Length != 32) throw PylonException.Decoding($"a {bytes.Length}-byte certificate hash");
                list.Add(bytes);
            }
            return (url, list.ToArray());
        }
    }

    /// <summary>Splits a WebTransport stream's bytes into its length-prefixed frames. Chunks can end anywhere.</summary>
    public sealed class StreamFrames
    {
        /// <summary>A frame larger than this ends the session.</summary>
        public const int MaxFrame = 64 * 1024 * 1024;

        byte[] _buffer = new byte[64 * 1024];
        int _length;

        /// <summary>Add a chunk and return the frames it completed.</summary>
        public System.Collections.Generic.List<byte[]> Push(byte[] chunk, int count)
        {
            if (_length + count > _buffer.Length)
            {
                var bigger = new byte[Math.Max(_buffer.Length * 2, _length + count)];
                Buffer.BlockCopy(_buffer, 0, bigger, 0, _length);
                _buffer = bigger;
            }
            Buffer.BlockCopy(chunk, 0, _buffer, _length, count);
            _length += count;
            var frames = new System.Collections.Generic.List<byte[]>();
            var at = 0;
            while (_length - at >= 4)
            {
                var len = (_buffer[at] << 24) | (_buffer[at + 1] << 16) | (_buffer[at + 2] << 8) | _buffer[at + 3];
                if (len < 0 || len > MaxFrame) throw PylonException.Decoding($"a {(uint)len}-byte stream frame");
                if (_length - at - 4 < len) break;
                var frame = new byte[len];
                Buffer.BlockCopy(_buffer, at + 4, frame, 0, len);
                frames.Add(frame);
                at += 4 + len;
            }
            if (at > 0)
            {
                Buffer.BlockCopy(_buffer, at, _buffer, 0, _length - at);
                _length -= at;
            }
            return frames;
        }
    }

    /// <summary>Shard tickets (<c>v1.&lt;payload&gt;.&lt;signature&gt;</c>, the payload base64url JSON with <c>exp</c> in Unix seconds).</summary>
    public static class ShardTicket
    {
        /// <summary>
        /// True when the ticket expires within <paramref name="marginSeconds"/>.
        /// A ticket that does not parse counts as not expired: the server decides.
        /// </summary>
        public static bool Expired(string ticket, DateTimeOffset? now = null, double marginSeconds = 5)
        {
            var parts = ticket.Split('.');
            if (parts.Length < 2) return false;
            var b64 = parts[1].Replace('-', '+').Replace('_', '/');
            while (b64.Length % 4 != 0) b64 += "=";
            try
            {
                var json = PylonValue.Parse(Encoding.UTF8.GetString(Convert.FromBase64String(b64)));
                var exp = json["exp"];
                if (!exp.IsNumber) return false;
                var t = (now ?? DateTimeOffset.UtcNow).ToUnixTimeMilliseconds() / 1000.0;
                return exp.AsDouble() <= t + marginSeconds;
            }
            catch (FormatException)
            {
                return false;
            }
            catch (PylonException)
            {
                return false;
            }
        }
    }
}
