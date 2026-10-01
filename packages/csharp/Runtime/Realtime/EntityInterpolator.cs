#nullable enable
using System;
using System.Collections.Generic;

namespace Pylon.Realtime
{
    /// <summary>An entity's state at one tick.</summary>
    public sealed class EntitySample
    {
        public double Tick { get; }
        public double X { get; }
        public double Y { get; }
        public double Z { get; }
        /// <summary>Component id to bytes. Shared between samples; do not change it.</summary>
        public IReadOnlyDictionary<byte, byte[]> Components { get; }

        internal EntitySample(double tick, double x, double y, double z, IReadOnlyDictionary<byte, byte[]> components)
        {
            Tick = tick;
            X = x;
            Y = y;
            Z = z;
            Components = components;
        }

        internal EntitySample At(double tick) => new EntitySample(tick, X, Y, Z, Components);
    }

    /// <summary>One entity as drawn at the render tick. The object is reused between updates.</summary>
    public sealed class InterpolatedEntity
    {
        public ulong Id { get; }
        public double X { get; internal set; }
        public double Y { get; internal set; }
        public double Z { get; internal set; }
        /// <summary>Components as of the sample at or before the render tick.</summary>
        public IReadOnlyDictionary<byte, byte[]> Components { get; internal set; }
        /// <summary>The samples around the render tick; the same sample when there is no later one.</summary>
        public EntitySample From { get; internal set; }
        public EntitySample To { get; internal set; }
        /// <summary>The fraction between <see cref="From"/> and <see cref="To"/>, for game data kept in components.</summary>
        public double T { get; internal set; }

        internal InterpolatedEntity(ulong id, EntitySample from, EntitySample to)
        {
            Id = id;
            From = from;
            To = to;
            Components = from.Components;
        }
    }

    /// <summary>
    /// Entity interpolation for render loops (a port of the TypeScript
    /// <c>EntityInterpolator</c>).
    ///
    /// <see cref="Record"/> every replication frame with its tick; each
    /// render frame, call <see cref="Update"/> with a tick a little behind
    /// the server's (<see cref="ShardClock.ServerTick"/> minus a delay).
    /// Each entity is drawn between the two samples around that tick.
    /// Spawns, despawns, and component changes appear at the tick they
    /// happened.
    /// </summary>
    public sealed class EntityInterpolator
    {
        sealed class Life
        {
            public double SpawnTick;
            public double? DespawnTick;
            public readonly List<EntitySample> Samples = new List<EntitySample>();
        }

        const int EndedTicks = 256;
        static readonly IReadOnlyDictionary<byte, byte[]> Empty = new Dictionary<byte, byte[]>();

        readonly Dictionary<ulong, InterpolatedEntity> _entities = new Dictionary<ulong, InterpolatedEntity>();
        readonly Dictionary<ulong, List<Life>> _lives = new Dictionary<ulong, List<Life>>();
        readonly Dictionary<ulong, Life> _shown = new Dictionary<ulong, Life>();
        readonly List<(ulong Id, Life Life)> _ended = new List<(ulong, Life)>();
        readonly double _snapDistance;
        readonly bool _holdWhenQuiet;
        readonly int _maxSamples;
        double _lastTick = -1;
        double _floor = double.NegativeInfinity;
        double _drawn = -1;

        /// <param name="snapDistance">A move longer than this (world units) between two samples is a teleport.</param>
        /// <param name="holdWhenQuiet">
        /// Treat an entity that sent no update as not moving, so its next move starts from the tick
        /// before it arrives. Set false for a shard with a byte budget, where updates can wait.
        /// </param>
        /// <param name="maxSamples">Samples kept per entity.</param>
        public EntityInterpolator(double snapDistance = double.PositiveInfinity, bool holdWhenQuiet = true, int maxSamples = 32)
        {
            _snapDistance = snapDistance;
            _holdWhenQuiet = holdWhenQuiet;
            _maxSamples = Math.Max(2, maxSamples);
        }

        /// <summary>Entities that exist at the last <see cref="Update"/>'s render tick.</summary>
        public IReadOnlyDictionary<ulong, InterpolatedEntity> Entities => _entities;

        /// <summary>
        /// Ids the last update added. An id in both <see cref="Left"/> and
        /// <see cref="Entered"/> is a new entity that reused the id.
        /// </summary>
        public List<ulong> Entered { get; } = new List<ulong>();

        /// <summary>Ids the last update removed.</summary>
        public List<ulong> Left { get; } = new List<ulong>();

        /// <summary>The newest tick recorded, or -1.</summary>
        public double LatestTick => _lastTick;

        /// <summary>
        /// The tick the last <see cref="Update"/> drew: its render tick, raised
        /// to what an earlier update drew and capped at the newest tick
        /// received, or -1 before the first update. This is the tick the
        /// player saw: the view tick for lag compensation.
        /// </summary>
        public double DrawnTick => _drawn;

        /// <summary>Forget everything. The next update removes every entity.</summary>
        public void Clear()
        {
            _lives.Clear();
            _ended.Clear();
            _lastTick = -1;
            _floor = double.NegativeInfinity;
            _drawn = -1;
        }

        /// <summary>Record the table after a frame for <paramref name="tick"/> applied, with the summary it returned.</summary>
        public void Record(EntityTable table, ReplicationSummary summary, double tick)
        {
            // Ticks went back: a restarted or different shard.
            if (tick < _lastTick) Clear();
            _lastTick = tick;
            DropEnded(tick);

            if (summary.Full)
            {
                // The table was rebuilt: what it lacks now is gone.
                var present = new HashSet<ulong>(summary.Spawned);
                foreach (var kv in _lives)
                {
                    var open = kv.Value[kv.Value.Count - 1];
                    if (open.DespawnTick == null && !present.Contains(kv.Key)) End(kv.Key, open, tick);
                }
                foreach (var id in summary.Spawned)
                {
                    var e = table.Get(id);
                    if (e == null) continue;
                    var open = OpenLife(id);
                    if (open != null) Push(open, e, tick);
                    else Spawn(id, e, tick);
                }
                return;
            }

            foreach (var id in summary.Despawned)
            {
                var open = OpenLife(id);
                if (open != null) End(id, open, tick);
            }
            foreach (var id in summary.Spawned)
            {
                var e = table.Get(id);
                if (e != null) Spawn(id, e, tick);
            }
            foreach (var id in summary.Updated)
            {
                var e = table.Get(id);
                var open = OpenLife(id);
                if (e != null && open != null) Push(open, e, tick);
            }
        }

        /// <summary>Place every entity at <paramref name="renderTick"/>. A render tick below one already drawn is raised to it.</summary>
        public void Update(double renderTick)
        {
            Entered.Clear();
            Left.Clear();
            renderTick = Math.Max(renderTick, _floor);
            _floor = Math.Min(renderTick, _lastTick);
            // Past the newest tick received, entities hold at their last
            // samples: the picture is that tick's.
            _drawn = _floor;
            var gone = new List<ulong>();
            foreach (var kv in _lives)
            {
                var id = kv.Key;
                var lives = kv.Value;
                while (lives.Count > 0 && lives[0].DespawnTick != null && lives[0].DespawnTick <= renderTick) lives.RemoveAt(0);
                if (lives.Count == 0)
                {
                    gone.Add(id);
                    continue;
                }
                var life = lives[0];
                if (life.SpawnTick > renderTick)
                {
                    Remove(id);
                    continue;
                }
                Place(id, life, renderTick);
            }
            foreach (var id in gone)
            {
                _lives.Remove(id);
                Remove(id);
            }
            if (_entities.Count > _lives.Count)
            {
                var stale = new List<ulong>();
                foreach (var id in _entities.Keys)
                {
                    if (!_lives.ContainsKey(id)) stale.Add(id);
                }
                foreach (var id in stale) Remove(id);
            }
        }

        void End(ulong id, Life life, double tick)
        {
            life.DespawnTick = tick;
            _ended.Add((id, life));
        }

        void DropEnded(double tick)
        {
            var n = 0;
            while (n < _ended.Count && _ended[n].Life.DespawnTick < tick - EndedTicks)
            {
                var (id, life) = _ended[n];
                if (_lives.TryGetValue(id, out var lives))
                {
                    lives.Remove(life);
                    if (lives.Count == 0) _lives.Remove(id);
                }
                n++;
            }
            if (n > 0) _ended.RemoveRange(0, n);
        }

        Life? OpenLife(ulong id)
        {
            if (!_lives.TryGetValue(id, out var lives) || lives.Count == 0) return null;
            var last = lives[lives.Count - 1];
            return last.DespawnTick == null ? last : null;
        }

        void Spawn(ulong id, ReplicatedEntity e, double tick)
        {
            if (!_lives.TryGetValue(id, out var lives))
            {
                lives = new List<Life>();
                _lives[id] = lives;
            }
            if (lives.Count > 0 && lives[lives.Count - 1].DespawnTick == null) End(id, lives[lives.Count - 1], tick);
            var life = new Life { SpawnTick = tick };
            life.Samples.Add(new EntitySample(tick, e.X, e.Y, e.Z, new Dictionary<byte, byte[]>(e.Components)));
            lives.Add(life);
        }

        void Push(Life life, ReplicatedEntity e, double tick)
        {
            var samples = life.Samples;
            var last = samples[samples.Count - 1];
            var components = SameComponents(last.Components, e.Components)
                ? last.Components
                : new Dictionary<byte, byte[]>(e.Components);
            var moved = last.X != e.X || last.Y != e.Y || last.Z != e.Z;
            if (!moved && ReferenceEquals(components, last.Components)) return;
            if (last.Tick == tick)
            {
                samples[samples.Count - 1] = new EntitySample(tick, e.X, e.Y, e.Z, components);
                return;
            }
            // No update since `last`: it stood still until the tick before.
            if (_holdWhenQuiet && moved && last.Tick < tick - 1) samples.Add(last.At(tick - 1));
            samples.Add(new EntitySample(tick, e.X, e.Y, e.Z, components));
            while (samples.Count > _maxSamples) samples.RemoveAt(0);
        }

        void Place(ulong id, Life life, double renderTick)
        {
            var samples = life.Samples;
            var i = samples.Count - 1;
            while (i > 0 && samples[i].Tick > renderTick) i--;
            var from = samples[i];
            var to = i + 1 < samples.Count ? samples[i + 1] : from;
            var t = ReferenceEquals(to, from) ? 0 : (renderTick - from.Tick) / (to.Tick - from.Tick);
            t = Math.Min(1, Math.Max(0, t));
            var dx = to.X - from.X;
            var dy = to.Y - from.Y;
            var dz = to.Z - from.Z;
            var teleport = dx * dx + dy * dy + dz * dz > _snapDistance * _snapDistance;
            var k = teleport ? 0 : t;

            _entities.TryGetValue(id, out var view);
            if (view != null && (!_shown.TryGetValue(id, out var shown) || !ReferenceEquals(shown, life)))
            {
                // The id names a new entity now: the old one left.
                Remove(id);
                view = null;
            }
            if (view == null)
            {
                view = new InterpolatedEntity(id, from, to);
                _entities[id] = view;
                _shown[id] = life;
                Entered.Add(id);
            }
            view.X = from.X + dx * k;
            view.Y = from.Y + dy * k;
            view.Z = from.Z + dz * k;
            view.Components = from.Components;
            view.From = from;
            view.To = to;
            view.T = t;

            // Samples two behind the one in use are no longer needed.
            if (i > 1) samples.RemoveRange(0, i - 1);
        }

        void Remove(ulong id)
        {
            _shown.Remove(id);
            if (_entities.Remove(id)) Left.Add(id);
        }

        static bool SameComponents(IReadOnlyDictionary<byte, byte[]> a, IReadOnlyDictionary<byte, byte[]> b)
        {
            if (a.Count != b.Count) return false;
            foreach (var kv in b)
            {
                if (!a.TryGetValue(kv.Key, out var w)) return false;
                if (ReferenceEquals(w, kv.Value)) continue;
                if (!PylonValue.BytesEqual(w, kv.Value)) return false;
            }
            return true;
        }
    }
}
