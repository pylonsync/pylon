#nullable enable
using System;
using System.Collections.Generic;

namespace Pylon.Realtime
{
    /// <summary>
    /// Client-side prediction for the local player.
    ///
    /// Apply each input locally as it is sent, and keep it until the shard
    /// acknowledges it. When a frame arrives with the server's state and its
    /// ack (the highest input sequence number the shard has processed),
    /// start from the server's state and apply the inputs it has not
    /// processed yet.
    ///
    /// <c>step</c> must be deterministic, match what the shard does with an
    /// input closely enough that the replayed state agrees with the
    /// server's, and return a new state instead of changing its argument.
    /// </summary>
    /// <summary>What <see cref="ShardGame{TInput}"/> needs from each predictor, whatever its state type.</summary>
    internal interface IInputLog<TInput>
    {
        void Push(ulong seq, TInput input);
        void Reject(ulong? seq);
        void Reset();
    }

    public sealed class Predictor<TState, TInput> : IInputLog<TInput>
    {
        readonly Func<TState, TInput, TState> _step;
        readonly int _maxPending;
        readonly List<(ulong Seq, TInput Input)> _pending = new List<(ulong, TInput)>();

        /// <param name="step">Applies one input to a state.</param>
        /// <param name="maxPending">Inputs kept while waiting for an ack; past this the oldest are dropped.</param>
        public Predictor(Func<TState, TInput, TState> step, int maxPending = 256)
        {
            _step = step ?? throw new ArgumentNullException(nameof(step));
            _maxPending = Math.Max(1, maxPending);
        }

        /// <summary>Inputs sent and not yet acknowledged.</summary>
        public int Count => _pending.Count;

        /// <summary>Record an input sent with sequence number <paramref name="seq"/>. A <paramref name="seq"/> of 0 (not sent) is ignored.</summary>
        public void Push(ulong seq, TInput input)
        {
            if (seq == 0) return;
            _pending.Add((seq, input));
            if (_pending.Count > _maxPending) _pending.RemoveAt(0);
        }

        /// <summary>The shard refused input <paramref name="seq"/>: it will never apply.</summary>
        public void Reject(ulong? seq)
        {
            if (seq == null) return;
            _pending.RemoveAll(p => p.Seq == seq.Value);
        }

        /// <summary>
        /// The predicted state: <paramref name="server"/> with every input
        /// after <paramref name="ack"/> applied. Inputs up to the ack are dropped.
        /// </summary>
        public TState Reconcile(TState server, ulong ack)
        {
            var drop = 0;
            while (drop < _pending.Count && _pending[drop].Seq <= ack) drop++;
            if (drop > 0) _pending.RemoveRange(0, drop);
            var state = server;
            foreach (var p in _pending) state = _step(state, p.Input);
            return state;
        }

        /// <summary>
        /// Drop every pending input. Call it when the connection reopens:
        /// inputs sent on the old connection either applied before it
        /// closed or never will.
        /// </summary>
        public void Reset() => _pending.Clear();
    }
}
