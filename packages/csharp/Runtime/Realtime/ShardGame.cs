#nullable enable
using System;
using System.Collections.Generic;

namespace Pylon.Realtime
{
    /// <summary>Settings for <see cref="ShardGame{TInput}"/> beyond the connection's.</summary>
    public sealed class ShardGameOptions
    {
        /// <summary>
        /// How far behind the shard's estimated current tick entities are
        /// drawn. It must cover the time between frames plus network jitter.
        /// Default 100 ms.
        /// </summary>
        public TimeSpan InterpolationDelay { get; set; } = TimeSpan.FromMilliseconds(100);

        /// <summary>A move longer than this (world units) between two samples is a teleport. Default: no limit.</summary>
        public double SnapDistance { get; set; } = double.PositiveInfinity;

        /// <summary>
        /// Treat an entity that sent no update as not moving. Default true.
        /// Set false for a shard with a byte budget, where updates can wait.
        /// </summary>
        public bool HoldWhenQuiet { get; set; } = true;

        /// <summary>Samples kept per entity. Default 32.</summary>
        public int MaxSamples { get; set; } = 32;
    }

    /// <summary>
    /// A shard client for a game's render loop: the connection, the clock,
    /// interpolated entities, and prediction (a port of the TypeScript
    /// <c>connectShardGame</c>).
    ///
    /// <code>
    /// var game = new ShardGame&lt;Move&gt;(shardId, options, moveConverter);
    /// var me = game.Predict&lt;Vector3&gt;((p, input) =&gt; Step(p, input));
    /// game.Replication += u =&gt; { if (u.Entities.Get(myId) is { } e) local = me.Reconcile(Pos(e), u.Ack); };
    /// game.Connect();
    ///
    /// void Update() {
    ///     game.Frame();                       // places game.Entities at the render tick
    ///     foreach (var e in game.Entities.Values) Draw(e.Id, e.X, e.Y, e.Z);
    ///     if (game.Send(input) != 0) local = Step(local, input); // 0: not sent, do not predict
    /// }
    /// </code>
    ///
    /// Events and <see cref="Frame"/> belong on one thread: the connection's
    /// dispatcher (Unity's main thread when created there).
    /// </summary>
    public sealed class ShardGame<TInput> : IDisposable
    {
        readonly IPylonConverter<TInput> _inputs;
        readonly EntityInterpolator _interpolator;
        readonly double _delayMs;
        readonly Func<double> _now;
        readonly List<IInputLog<TInput>> _predictors = new List<IInputLog<TInput>>();

        /// <summary>The underlying connection.</summary>
        public ShardConnection Connection { get; }

        public ShardGame(
            string shardId,
            ShardConnectionOptions options,
            IPylonConverter<TInput> inputs,
            ShardGameOptions? game = null)
        {
            _inputs = inputs ?? throw new ArgumentNullException(nameof(inputs));
            game ??= new ShardGameOptions();
            _interpolator = new EntityInterpolator(game.SnapDistance, game.HoldWhenQuiet, game.MaxSamples);
            _delayMs = game.InterpolationDelay.TotalMilliseconds;
            _now = options.Now ?? ShardClock.Now;
            // WebTransport where the plugin and the app support it, as in the TypeScript client.
            Connection = new ShardConnection(shardId, options, ShardTransport.Auto);
            Connection.Replication += u => _interpolator.Record(u.Entities, u.Summary, u.Tick);
            Connection.InputRejected += r =>
            {
                foreach (var p in _predictors) p.Reject(r.ClientSeq);
            };
            Connection.Opened += () =>
            {
                foreach (var p in _predictors) p.Reset();
            };
            // Another shard: its entities and ticks start over.
            Connection.Transferred += (_, _) => _interpolator.Clear();
        }

        public ShardClock Clock => Connection.Clock;

        /// <summary>Entities placed at the render tick by the last <see cref="Frame"/> call.</summary>
        public IReadOnlyDictionary<ulong, InterpolatedEntity> Entities => _interpolator.Entities;

        /// <summary>Ids the last <see cref="Frame"/> added to <see cref="Entities"/>.</summary>
        public IReadOnlyList<ulong> Entered => _interpolator.Entered;

        /// <summary>Ids the last <see cref="Frame"/> removed from <see cref="Entities"/>.</summary>
        public IReadOnlyList<ulong> Left => _interpolator.Left;

        /// <summary>The newest state the shard sent, not interpolated.</summary>
        public EntityTable Latest => Connection.Entities;

        public ulong Tick => Connection.Tick;
        public ulong Ack => Connection.Ack;
        public double? RttMs => Connection.RttMs;
        public bool Connected => Connection.Connected;
        public string ShardId => Connection.ShardId;

        public event Action<ShardReplicationUpdate>? Replication
        {
            add => Connection.Replication += value;
            remove => Connection.Replication -= value;
        }

        public event Action<ShardInputRejection>? InputRejected
        {
            add => Connection.InputRejected += value;
            remove => Connection.InputRejected -= value;
        }

        /// <summary>The server moved this player to another shard: (new shard, previous shard).</summary>
        public event Action<string, string>? Transferred
        {
            add => Connection.Transferred += value;
            remove => Connection.Transferred -= value;
        }

        public event Action? Opened
        {
            add => Connection.Opened += value;
            remove => Connection.Opened -= value;
        }

        public event Action<ShardCloseInfo>? Closed
        {
            add => Connection.Closed += value;
            remove => Connection.Closed -= value;
        }

        public event Action<ShardConnectionState, string?>? StateChanged
        {
            add => Connection.StateChanged += value;
            remove => Connection.StateChanged -= value;
        }

        public event Action<Exception>? Error
        {
            add => Connection.Error += value;
            remove => Connection.Error -= value;
        }

        /// <summary>Open the connection.</summary>
        public void Connect() => Connection.Connect();

        /// <summary>
        /// Place <see cref="Entities"/> for a render frame at <paramref name="now"/>
        /// (milliseconds on the connection's clock; default now). Returns the
        /// render tick, or -1 before the first frame.
        /// </summary>
        public double Frame(double? now = null)
        {
            var clock = Connection.Clock;
            if (!clock.Ready) return -1;
            var renderTick = clock.ServerTick(now ?? _now()) - _delayMs / clock.TickMs;
            _interpolator.Update(renderTick);
            return renderTick;
        }

        /// <summary>
        /// Send an input; every predictor made by <see cref="Predict{TState}"/>
        /// records it. Returns its sequence number, or 0 when it was not sent.
        /// </summary>
        public ulong Send(TInput input)
        {
            var seq = Connection.Send(_inputs.ToValue(input));
            foreach (var p in _predictors) p.Push(seq, input);
            return seq;
        }

        /// <summary>
        /// A predictor that records every input <see cref="Send"/> sends,
        /// forgets inputs the shard refuses, and resets when the connection
        /// reopens. Call <c>Reconcile</c> with the local entity's server state
        /// after each frame.
        /// </summary>
        public Predictor<TState, TInput> Predict<TState>(Func<TState, TInput, TState> step, int maxPending = 256)
        {
            var p = new Predictor<TState, TInput>(step, maxPending);
            _predictors.Add(p);
            return p;
        }

        public void Dispose() => Connection.Dispose();
    }
}
