#nullable enable
using System;
using System.Collections.Generic;

namespace Pylon.Realtime
{
    /// <summary>A replication frame or datagram that does not decode. Reconnect: the server then sends a full frame.</summary>
    public sealed class ReplicationException : Exception
    {
        public ReplicationException(string message) : base("replication frame: " + message)
        {
        }
    }

    /// <summary>One entity as the client sees it.</summary>
    public sealed class ReplicatedEntity
    {
        public ulong Id { get; }
        /// <summary>Quantized position; <see cref="X"/>/<see cref="Y"/>/<see cref="Z"/> are these times the frame precision.</summary>
        public long QX { get; internal set; }
        public long QY { get; internal set; }
        public long QZ { get; internal set; }
        public double X { get; internal set; }
        public double Y { get; internal set; }
        public double Z { get; internal set; }
        /// <summary>Component id to bytes, in the game's own encoding. Do not change it.</summary>
        public Dictionary<byte, byte[]> Components { get; }

        internal ReplicatedEntity(ulong id, long qx, long qy, long qz, Dictionary<byte, byte[]> components)
        {
            Id = id;
            QX = qx;
            QY = qy;
            QZ = qz;
            Components = components;
        }
    }

    /// <summary>What one frame did.</summary>
    public sealed class ReplicationSummary
    {
        public bool Full { get; internal set; }
        public List<ulong> Spawned { get; } = new List<ulong>();
        public List<ulong> Updated { get; } = new List<ulong>();
        public List<ulong> Despawned { get; } = new List<ulong>();
    }

    /// <summary>What one datagram did.</summary>
    public sealed class DatagramSummary
    {
        public ulong Frame { get; internal set; }
        public ulong Tick { get; internal set; }
        public ulong Ack { get; internal set; }
        /// <summary>The tick of the last stream frame the server had sent by <see cref="Tick"/>.</summary>
        public ulong StreamTick { get; internal set; }
        /// <summary>Datagrams the server sent for <see cref="Tick"/>.</summary>
        public ulong Parts { get; internal set; }
        public List<ulong> Updated { get; } = new List<ulong>();
        /// <summary>Updates skipped: an unknown entity, another spawn, an older datagram, or another precision.</summary>
        public int Skipped { get; internal set; }
    }

    /// <summary>
    /// The entities one subscriber has been told about (see
    /// <c>pylon_replication::frame</c> in Rust). Apply every replication
    /// frame in order. A frame that fails to apply means the connection is
    /// out of sync: clear the table and reconnect, and the server sends a
    /// full frame.
    /// <code>
    /// u8      version (1)
    /// u8      flags: bit 0 FULL: clear the table first
    /// f32 LE  precision
    /// varint  despawn count, ids (first absolute, then differences; ascending)
    /// varint  spawn count, per entity: id, x, y, z (zigzag, quantized), components
    /// varint  update count, per entity: id, u8 mask (1 x, 2 y, 4 z, 8 components),
    ///         changed axes (zigzag differences), components if bit 8
    /// components: varint count, per component: u8 id, varint (length + 1),
    ///         bytes; length field 0 means removed
    /// </code>
    /// Datagrams (WebTransport, version 2) carry absolute positions and the
    /// low 16 bits of the spawn tick; <see cref="ApplyDatagram"/> applies one
    /// only to the entity spawned at that tick and only when it is newer.
    /// </summary>
    public sealed class EntityTable
    {
        public const byte ReplicationVersion = 1;
        public const byte DatagramVersion = 2;

        const byte FlagFull = 1;
        const byte MaskX = 1;
        const byte MaskY = 2;
        const byte MaskZ = 4;
        const byte MaskComponents = 8;
        const ulong MaxCount = 1 << 24;

        readonly Dictionary<ulong, ReplicatedEntity> _entities = new Dictionary<ulong, ReplicatedEntity>();
        readonly Dictionary<ulong, ulong> _spawnTicks = new Dictionary<ulong, ulong>();
        readonly Dictionary<ulong, ulong> _datagramFrames = new Dictionary<ulong, ulong>();

        public IReadOnlyDictionary<ulong, ReplicatedEntity> Entities => _entities;

        /// <summary>World units per quantization step, from the last frame.</summary>
        public float Precision { get; private set; } = 0.01f;

        /// <summary>The tick of the last frame applied with its tick. Datagram acks name it.</summary>
        public ulong StreamTick { get; private set; }

        public int Count => _entities.Count;

        public ReplicatedEntity? Get(ulong id) => _entities.TryGetValue(id, out var e) ? e : null;

        /// <summary>The tick of the stream frame that spawned an entity, when frames were applied with their tick.</summary>
        public ulong? SpawnTick(ulong id) => _spawnTicks.TryGetValue(id, out var t) ? (ulong?)t : null;

        public void Clear()
        {
            _entities.Clear();
            _spawnTicks.Clear();
            _datagramFrames.Clear();
            StreamTick = 0;
        }

        /// <summary>True when a replication frame is a full frame (it replaces the table).</summary>
        public static bool IsFullFrame(ArraySegment<byte> frame) =>
            frame.Count >= 2 && (frame.Array![frame.Offset + 1] & FlagFull) != 0;

        public ReplicationSummary Apply(byte[] frame, ulong? tick = null) => Apply(new ArraySegment<byte>(frame), tick);

        /// <summary>
        /// Apply a frame. Pass the tick from the frame header on a
        /// connection that also gets datagrams; the table then records each
        /// entity's spawn tick and the last frame's tick.
        /// </summary>
        public ReplicationSummary Apply(ArraySegment<byte> frame, ulong? tick = null)
        {
            var r = new Reader(frame);
            var version = r.U8();
            if (version != ReplicationVersion) throw new ReplicationException($"version {version}");
            var full = (r.U8() & FlagFull) != 0;
            var precision = r.F32();
            if (float.IsNaN(precision) || float.IsInfinity(precision) || precision <= 0)
                throw new ReplicationException("bad precision");
            if (full)
            {
                _entities.Clear();
                _datagramFrames.Clear();
                _spawnTicks.Clear();
            }
            Precision = precision;
            var summary = new ReplicationSummary { Full = full };

            var n = ReadCount(r);
            ulong? last = null;
            for (ulong i = 0; i < n; i++)
            {
                var id = NextId(r, last);
                last = id;
                _entities.Remove(id);
                _datagramFrames.Remove(id);
                _spawnTicks.Remove(id);
                summary.Despawned.Add(id);
            }

            n = ReadCount(r);
            last = null;
            for (ulong i = 0; i < n; i++)
            {
                var id = NextId(r, last);
                last = id;
                var qx = r.ZigZag();
                var qy = r.ZigZag();
                var qz = r.ZigZag();
                var components = new Dictionary<byte, byte[]>();
                ReadComponents(r, components);
                _entities[id] = new ReplicatedEntity(id, qx, qy, qz, components);
                _datagramFrames.Remove(id);
                if (tick.HasValue) _spawnTicks[id] = tick.Value;
                else _spawnTicks.Remove(id);
                summary.Spawned.Add(id);
            }

            n = ReadCount(r);
            last = null;
            for (ulong i = 0; i < n; i++)
            {
                var id = NextId(r, last);
                last = id;
                var mask = r.U8();
                if (!_entities.TryGetValue(id, out var e)) throw new ReplicationException($"update for unknown entity {id}");
                if ((mask & MaskX) != 0) e.QX = Add(e.QX, r.ZigZag());
                if ((mask & MaskY) != 0) e.QY = Add(e.QY, r.ZigZag());
                if ((mask & MaskZ) != 0) e.QZ = Add(e.QZ, r.ZigZag());
                if ((mask & MaskComponents) != 0) ReadComponents(r, e.Components);
                summary.Updated.Add(id);
            }
            if (!r.Done) throw new ReplicationException("trailing bytes");

            // Positions follow the frame's precision (a full frame may change it).
            foreach (var e in _entities.Values) Place(e, precision);
            if (tick.HasValue) StreamTick = tick.Value;
            return summary;
        }

        public DatagramSummary ApplyDatagram(byte[] datagram) => ApplyDatagram(new ArraySegment<byte>(datagram));

        /// <summary>
        /// Apply one datagram. It changes nothing it cannot apply exactly
        /// (see <see cref="DatagramSummary.Skipped"/>). Throws only on bytes
        /// that are not a datagram.
        /// </summary>
        public DatagramSummary ApplyDatagram(ArraySegment<byte> datagram)
        {
            var r = new Reader(datagram);
            var version = r.U8();
            if (version != DatagramVersion) throw new ReplicationException($"datagram version {version}");
            var summary = new DatagramSummary
            {
                Frame = r.Varint(),
                Tick = r.Varint(),
                Ack = r.Varint(),
                StreamTick = r.Varint(),
            };
            var precision = r.F32();
            summary.Parts = r.Varint();
            var n = r.Varint();
            if (n > 1 << 16) throw new ReplicationException("count too large");
            // Positions in another precision mean nothing to this table: the
            // full frame of the new precision is still on its way.
            var usable = precision == Precision;
            ulong? last = null;
            for (ulong i = 0; i < n; i++)
            {
                var id = NextId(r, last);
                last = id;
                var tag = r.U8() | (r.U8() << 8);
                var mask = r.U8();
                long? qx = (mask & MaskX) != 0 ? r.ZigZag() : (long?)null;
                long? qy = (mask & MaskY) != 0 ? r.ZigZag() : (long?)null;
                long? qz = (mask & MaskZ) != 0 ? r.ZigZag() : (long?)null;
                List<KeyValuePair<byte, byte[]>>? changes = null;
                List<byte>? removed = null;
                if ((mask & MaskComponents) != 0)
                {
                    var count = r.Varint();
                    if (count > 256) throw new ReplicationException("more than 256 components");
                    for (ulong c = 0; c < count; c++)
                    {
                        var cid = r.U8();
                        var len = r.Varint();
                        if (len == 0)
                        {
                            (removed ??= new List<byte>()).Add(cid);
                            continue;
                        }
                        (changes ??= new List<KeyValuePair<byte, byte[]>>()).Add(
                            new KeyValuePair<byte, byte[]>(cid, r.Take(len - 1)));
                    }
                }
                _entities.TryGetValue(id, out var e);
                var hasLast = _datagramFrames.TryGetValue(id, out var lastFrame);
                var current = usable && e != null && _spawnTicks.TryGetValue(id, out var spawn) &&
                              (int)(spawn % 0x10000) == tag && (!hasLast || summary.Frame > lastFrame);
                if (!current || e == null)
                {
                    summary.Skipped++;
                    continue;
                }
                if (qx.HasValue) e.QX = qx.Value;
                if (qy.HasValue) e.QY = qy.Value;
                if (qz.HasValue) e.QZ = qz.Value;
                Place(e, Precision);
                if (removed != null) foreach (var cid in removed) e.Components.Remove(cid);
                if (changes != null) foreach (var kv in changes) e.Components[kv.Key] = kv.Value;
                _datagramFrames[id] = summary.Frame;
                summary.Updated.Add(id);
            }
            if (!r.Done) throw new ReplicationException("trailing bytes");
            return summary;
        }

        /// <summary>A datagram's header fields, without applying it.</summary>
        public static DatagramSummary ReadDatagramHeader(ArraySegment<byte> datagram)
        {
            var r = new Reader(datagram);
            var version = r.U8();
            if (version != DatagramVersion) throw new ReplicationException($"datagram version {version}");
            var s = new DatagramSummary { Frame = r.Varint(), Tick = r.Varint(), Ack = r.Varint(), StreamTick = r.Varint() };
            r.F32();
            s.Parts = r.Varint();
            return s;
        }

        static void Place(ReplicatedEntity e, float precision)
        {
            e.X = e.QX * (double)precision;
            e.Y = e.QY * (double)precision;
            e.Z = e.QZ * (double)precision;
        }

        static long Add(long a, long b)
        {
            try
            {
                return checked(a + b);
            }
            catch (OverflowException)
            {
                throw new ReplicationException("position out of range");
            }
        }

        static ulong ReadCount(Reader r)
        {
            var n = r.Varint();
            if (n > MaxCount) throw new ReplicationException("count too large");
            return n;
        }

        static ulong NextId(Reader r, ulong? last)
        {
            var v = r.Varint();
            if (last == null) return v;
            if (v == 0) throw new ReplicationException("ids do not ascend");
            try
            {
                return checked(last.Value + v);
            }
            catch (OverflowException)
            {
                throw new ReplicationException("entity id out of range");
            }
        }

        static void ReadComponents(Reader r, Dictionary<byte, byte[]> into)
        {
            var n = r.Varint();
            if (n > 256) throw new ReplicationException("more than 256 components");
            for (ulong i = 0; i < n; i++)
            {
                var id = r.U8();
                var len = r.Varint();
                if (len == 0)
                {
                    into.Remove(id);
                    continue;
                }
                into[id] = r.Take(len - 1);
            }
        }

        sealed class Reader
        {
            readonly byte[] _b;
            readonly int _end;
            int _at;

            public Reader(ArraySegment<byte> seg)
            {
                _b = seg.Array ?? new byte[0];
                _at = seg.Offset;
                _end = seg.Offset + seg.Count;
            }

            public bool Done => _at >= _end;

            public byte U8()
            {
                if (_at >= _end) throw new ReplicationException("ends early");
                return _b[_at++];
            }

            public float F32()
            {
                if (_end - _at < 4) throw new ReplicationException("ends early");
                var bits = _b[_at] | (_b[_at + 1] << 8) | (_b[_at + 2] << 16) | (_b[_at + 3] << 24);
                _at += 4;
                return BitConverter.Int32BitsToSingle(bits);
            }

            /// <summary>An unsigned LEB128 varint, exact to 64 bits.</summary>
            public ulong Varint()
            {
                ulong v = 0;
                for (var i = 0; i < 10; i++)
                {
                    var b = U8();
                    var part = (ulong)(b & 0x7f);
                    if (i == 9 && part > 1) throw new ReplicationException("varint past 64 bits");
                    v |= part << (7 * i);
                    if ((b & 0x80) == 0) return v;
                }
                throw new ReplicationException("varint too long");
            }

            /// <summary>A zigzag signed varint.</summary>
            public long ZigZag()
            {
                var z = Varint();
                return (long)(z >> 1) ^ -(long)(z & 1);
            }

            public byte[] Take(ulong n)
            {
                if (n > (ulong)(_end - _at)) throw new ReplicationException("component runs past the end");
                var out_ = new byte[n];
                Buffer.BlockCopy(_b, _at, out_, 0, (int)n);
                _at += (int)n;
                return out_;
            }
        }
    }
}
