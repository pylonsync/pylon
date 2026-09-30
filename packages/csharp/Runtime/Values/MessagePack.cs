#nullable enable
using System;
using System.Collections.Generic;
using System.IO;
using System.Text;

namespace Pylon
{
    /// <summary>
    /// MessagePack encoding of <see cref="PylonValue"/>, for shards with the
    /// <c>msgpack</c> codec. Written without reflection, so it works under
    /// IL2CPP. Integers use the smallest encoding; floats are written as
    /// float64. Timestamps and other extension types are refused.
    /// </summary>
    public static class MessagePack
    {
        const int MaxDepth = 512;

        public static byte[] Encode(PylonValue value)
        {
            using var ms = new MemoryStream();
            Write(ms, value ?? PylonValue.Null, 0);
            return ms.ToArray();
        }

        /// <summary>Decode one value that fills <paramref name="bytes"/>.</summary>
        public static PylonValue Decode(byte[] bytes) => Decode(bytes, 0, bytes.Length);

        public static PylonValue Decode(byte[] bytes, int offset, int count)
        {
            if (bytes == null) throw new ArgumentNullException(nameof(bytes));
            if (offset < 0 || count < 0 || offset + count > bytes.Length) throw new ArgumentOutOfRangeException(nameof(count));
            var r = new Reader(bytes, offset, offset + count);
            var value = r.Read(0);
            if (r.Position != offset + count) throw PylonException.Decoding("msgpack: trailing bytes");
            return value;
        }

        // ---- encoding ----

        static void Write(Stream s, PylonValue v, int depth)
        {
            if (depth > MaxDepth) throw PylonException.InvalidArgument("value nested too deeply to encode");
            switch (v.Kind)
            {
                case PylonValueKind.Null:
                    s.WriteByte(0xc0);
                    break;
                case PylonValueKind.Bool:
                    s.WriteByte(v.AsBool() ? (byte)0xc3 : (byte)0xc2);
                    break;
                case PylonValueKind.Int:
                    WriteInt(s, v.LongUnsafe);
                    break;
                case PylonValueKind.UInt:
                    WriteUInt(s, v.ULongUnsafe);
                    break;
                case PylonValueKind.Float:
                    s.WriteByte(0xcb);
                    WriteBigEndian(s, (ulong)BitConverter.DoubleToInt64Bits(v.DoubleUnsafe), 8);
                    break;
                case PylonValueKind.String:
                    var utf8 = Encoding.UTF8.GetBytes(v.StringUnsafe);
                    if (utf8.Length < 32) s.WriteByte((byte)(0xa0 | utf8.Length));
                    else WriteLength(s, utf8.Length, 0xd9, 0xda, 0xdb);
                    s.Write(utf8, 0, utf8.Length);
                    break;
                case PylonValueKind.Bytes:
                    var bytes = v.BytesUnsafe;
                    WriteLength(s, bytes.Length, 0xc4, 0xc5, 0xc6);
                    s.Write(bytes, 0, bytes.Length);
                    break;
                case PylonValueKind.Array:
                    var items = v.Items;
                    if (items.Count < 16) s.WriteByte((byte)(0x90 | items.Count));
                    else if (items.Count <= ushort.MaxValue)
                    {
                        s.WriteByte(0xdc);
                        WriteBigEndian(s, (ulong)items.Count, 2);
                    }
                    else
                    {
                        s.WriteByte(0xdd);
                        WriteBigEndian(s, (ulong)items.Count, 4);
                    }
                    foreach (var item in items) Write(s, item, depth + 1);
                    break;
                case PylonValueKind.Object:
                    var fields = v.Fields;
                    if (fields.Count < 16) s.WriteByte((byte)(0x80 | fields.Count));
                    else if (fields.Count <= ushort.MaxValue)
                    {
                        s.WriteByte(0xde);
                        WriteBigEndian(s, (ulong)fields.Count, 2);
                    }
                    else
                    {
                        s.WriteByte(0xdf);
                        WriteBigEndian(s, (ulong)fields.Count, 4);
                    }
                    foreach (var f in fields)
                    {
                        Write(s, PylonValue.From(f.Key), depth + 1);
                        Write(s, f.Value, depth + 1);
                    }
                    break;
            }
        }

        static void WriteInt(Stream s, long v)
        {
            if (v >= 0)
            {
                WriteUInt(s, (ulong)v);
                return;
            }
            if (v >= -32) s.WriteByte((byte)(sbyte)v);
            else if (v >= sbyte.MinValue)
            {
                s.WriteByte(0xd0);
                s.WriteByte((byte)(sbyte)v);
            }
            else if (v >= short.MinValue)
            {
                s.WriteByte(0xd1);
                WriteBigEndian(s, (ulong)(ushort)(short)v, 2);
            }
            else if (v >= int.MinValue)
            {
                s.WriteByte(0xd2);
                WriteBigEndian(s, (uint)(int)v, 4);
            }
            else
            {
                s.WriteByte(0xd3);
                WriteBigEndian(s, (ulong)v, 8);
            }
        }

        static void WriteUInt(Stream s, ulong v)
        {
            if (v < 128) s.WriteByte((byte)v);
            else if (v <= byte.MaxValue)
            {
                s.WriteByte(0xcc);
                s.WriteByte((byte)v);
            }
            else if (v <= ushort.MaxValue)
            {
                s.WriteByte(0xcd);
                WriteBigEndian(s, v, 2);
            }
            else if (v <= uint.MaxValue)
            {
                s.WriteByte(0xce);
                WriteBigEndian(s, v, 4);
            }
            else
            {
                s.WriteByte(0xcf);
                WriteBigEndian(s, v, 8);
            }
        }

        static void WriteLength(Stream s, int length, byte tag8, byte tag16, byte tag32)
        {
            if (length <= byte.MaxValue)
            {
                s.WriteByte(tag8);
                s.WriteByte((byte)length);
            }
            else if (length <= ushort.MaxValue)
            {
                s.WriteByte(tag16);
                WriteBigEndian(s, (ulong)length, 2);
            }
            else
            {
                s.WriteByte(tag32);
                WriteBigEndian(s, (ulong)length, 4);
            }
        }

        static void WriteBigEndian(Stream s, ulong v, int bytes)
        {
            for (var i = bytes - 1; i >= 0; i--) s.WriteByte((byte)(v >> (8 * i)));
        }

        // ---- decoding ----

        sealed class Reader
        {
            readonly byte[] _b;
            readonly int _end;
            public int Position;

            public Reader(byte[] b, int start, int end)
            {
                _b = b;
                Position = start;
                _end = end;
            }

            static PylonException Error(string message) => PylonException.Decoding("msgpack: " + message);

            byte U8()
            {
                if (Position >= _end) throw Error("ends early");
                return _b[Position++];
            }

            ulong BigEndian(int n)
            {
                if (_end - Position < n) throw Error("ends early");
                ulong v = 0;
                for (var i = 0; i < n; i++) v = (v << 8) | _b[Position++];
                return v;
            }

            int Length(int n)
            {
                var v = BigEndian(n);
                if (v > int.MaxValue || (long)v > _end - Position) throw Error("length runs past the end");
                return (int)v;
            }

            byte[] Take(int n)
            {
                if (n > _end - Position) throw Error("ends early");
                var out_ = new byte[n];
                Buffer.BlockCopy(_b, Position, out_, 0, n);
                Position += n;
                return out_;
            }

            string Str(int n)
            {
                if (n > _end - Position) throw Error("ends early");
                string s;
                try
                {
                    s = new UTF8Encoding(false, true).GetString(_b, Position, n);
                }
                catch (DecoderFallbackException)
                {
                    throw Error("a string is not UTF-8");
                }
                Position += n;
                return s;
            }

            public PylonValue Read(int depth)
            {
                if (depth > MaxDepth) throw Error("nested too deeply");
                var t = U8();
                if (t <= 0x7f) return PylonValue.From((long)t);
                if (t >= 0xe0) return PylonValue.From((long)(sbyte)t);
                if ((t & 0xf0) == 0x80) return Map(t & 0x0f, depth);
                if ((t & 0xf0) == 0x90) return Arr(t & 0x0f, depth);
                if ((t & 0xe0) == 0xa0) return PylonValue.From(Str(t & 0x1f));
                switch (t)
                {
                    case 0xc0: return PylonValue.Null;
                    case 0xc2: return PylonValue.False;
                    case 0xc3: return PylonValue.True;
                    case 0xc4: return PylonValue.OwnBytes(Take(Length(1)));
                    case 0xc5: return PylonValue.OwnBytes(Take(Length(2)));
                    case 0xc6: return PylonValue.OwnBytes(Take(Length(4)));
                    case 0xca:
                        return PylonValue.From((double)BitConverter.Int32BitsToSingle((int)(uint)BigEndian(4)));
                    case 0xcb:
                        return PylonValue.From(BitConverter.Int64BitsToDouble((long)BigEndian(8)));
                    case 0xcc: return PylonValue.From((long)BigEndian(1));
                    case 0xcd: return PylonValue.From((long)BigEndian(2));
                    case 0xce: return PylonValue.From((long)BigEndian(4));
                    case 0xcf: return PylonValue.From(BigEndian(8));
                    case 0xd0: return PylonValue.From((long)(sbyte)BigEndian(1));
                    case 0xd1: return PylonValue.From((long)(short)BigEndian(2));
                    case 0xd2: return PylonValue.From((long)(int)BigEndian(4));
                    case 0xd3: return PylonValue.From((long)BigEndian(8));
                    case 0xd9: return PylonValue.From(Str(Length(1)));
                    case 0xda: return PylonValue.From(Str(Length(2)));
                    case 0xdb: return PylonValue.From(Str(Length(4)));
                    case 0xdc: return Arr((int)BigEndian(2), depth);
                    case 0xdd: return Arr(Length(4), depth);
                    case 0xde: return Map((int)BigEndian(2), depth);
                    case 0xdf: return Map(Length(4), depth);
                    default:
                        throw Error($"unsupported type byte 0x{t:x2}");
                }
            }

            PylonValue Arr(int n, int depth)
            {
                // Every item takes at least one byte.
                if (n > _end - Position) throw Error("array runs past the end");
                var items = new PylonValue[n];
                for (var i = 0; i < n; i++) items[i] = Read(depth + 1);
                return PylonValue.OwnArray(items);
            }

            PylonValue Map(int n, int depth)
            {
                if (n > (_end - Position) / 2) throw Error("map runs past the end");
                var fields = new List<KeyValuePair<string, PylonValue>>(n);
                for (var i = 0; i < n; i++)
                {
                    var key = Read(depth + 1);
                    string name;
                    switch (key.Kind)
                    {
                        case PylonValueKind.String:
                            name = key.StringUnsafe;
                            break;
                        case PylonValueKind.Int:
                        case PylonValueKind.UInt:
                            // Maps keyed by integers (rmp-serde writes them for
                            // integer-keyed maps) read as their decimal keys.
                            name = key.ToJson();
                            break;
                        default:
                            throw Error("a map key is not a string or an integer");
                    }
                    fields.Add(new KeyValuePair<string, PylonValue>(name, Read(depth + 1)));
                }
                return PylonValue.OwnObject(fields);
            }
        }
    }
}
