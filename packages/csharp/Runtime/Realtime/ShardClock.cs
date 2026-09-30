#nullable enable
using System;
using System.Collections.Generic;
using System.Diagnostics;

namespace Pylon.Realtime
{
    /// <summary>
    /// An estimate of the shard's current tick, from the ticks in the frames
    /// it sends and when they arrive (a port of the TypeScript
    /// <c>ShardClock</c>).
    ///
    /// The frame that arrived soonest after it was built anchors the
    /// estimate, so network jitter only ever makes a frame look late. The
    /// tick length is the shard's tick rate when given, or else the median
    /// slope of arrival time against tick. Corrections are spread over time
    /// instead of jumping; a step forward larger than <c>snapMs</c> applies
    /// at once, and a step back never goes below the newest tick received.
    ///
    /// Times are milliseconds on one monotonic clock; <see cref="Now"/> is one.
    /// </summary>
    public sealed class ShardClock
    {
        const double DefaultTickMs = 50;
        const int LateFrames = 3;
        const int MinFitSamples = 8;

        static readonly Stopwatch Monotonic = Stopwatch.StartNew();

        /// <summary>Milliseconds on a monotonic clock, for <see cref="Observe"/> and <see cref="ServerTick"/>.</summary>
        public static double Now() => Monotonic.Elapsed.TotalMilliseconds;

        readonly double? _fixedTickMs;
        readonly int _window;
        readonly double _slew;
        readonly double _snapMs;
        readonly List<(double Tick, double At)> _samples = new List<(double, double)>();
        double _measuredTickMs = DefaultTickMs;
        double _anchorTick = -1;
        double _anchorAt;
        double _offset;
        double _slewedTo;
        int _late;

        /// <param name="tickRate">The shard's tick rate in ticks per second, when known.</param>
        /// <param name="window">Frames the estimate uses.</param>
        /// <param name="slew">The least fraction of real time the estimate speeds up or slows down by to absorb a correction.</param>
        /// <param name="snapMs">A step forward larger than this applies at once, and a correction this large runs at full rate.</param>
        public ShardClock(double? tickRate = null, int window = 64, double slew = 0.05, double snapMs = 250)
        {
            _fixedTickMs = tickRate.HasValue && tickRate.Value > 0 && !double.IsInfinity(tickRate.Value)
                ? 1000.0 / tickRate.Value
                : (double?)null;
            _window = Math.Max(2, window);
            _slew = Math.Min(0.5, Math.Max(0, slew));
            _snapMs = snapMs;
        }

        /// <summary>True once a frame has arrived.</summary>
        public bool Ready => _anchorTick >= 0;

        /// <summary>The newest tick a frame carried, or -1.</summary>
        public double LatestTick => _anchorTick;

        /// <summary>Milliseconds per tick: the configured rate, or the measured one.</summary>
        public double TickMs => _fixedTickMs ?? _measuredTickMs;

        /// <summary>Forget everything (a new shard).</summary>
        public void Reset()
        {
            _samples.Clear();
            _measuredTickMs = DefaultTickMs;
            _anchorTick = -1;
            _anchorAt = 0;
            _offset = 0;
            _slewedTo = 0;
            _late = 0;
        }

        /// <summary>Record a frame for <paramref name="tick"/> that arrived at <paramref name="at"/>.</summary>
        public void Observe(double tick, double at)
        {
            // Ticks went back: the shard restarted, or this is another shard.
            if (tick < _anchorTick) Reset();
            double? shown = Ready ? ServerTick(at) : (double?)null;
            var newestBefore = _anchorTick;

            // Several frames in a row late by more than snapMs, spaced like
            // ticks: the shard's schedule moved, so older samples go.
            if (Ready && _samples.Count > 0)
            {
                var prev = _samples[_samples.Count - 1];
                if (tick > prev.Tick)
                {
                    var expected = _anchorAt + (tick - _anchorTick) * TickMs;
                    var spaced = at - prev.At >= 0.5 * TickMs * (tick - prev.Tick);
                    _late = spaced && at - expected > _snapMs ? _late + 1 : 0;
                    if (_late >= LateFrames)
                    {
                        _samples.RemoveRange(0, Math.Max(0, _samples.Count - (LateFrames - 1)));
                        _late = 0;
                    }
                }
            }

            if (_samples.Count > 0 && _samples[_samples.Count - 1].Tick == tick)
            {
                // Another frame for the same tick: keep the earlier arrival.
                if (at >= _samples[_samples.Count - 1].At) return;
                _samples[_samples.Count - 1] = (tick, at);
            }
            else
            {
                _samples.Add((tick, at));
                if (_samples.Count > _window) _samples.RemoveAt(0);
            }
            if (_fixedTickMs == null) FitTickMs();

            var tickMs = TickMs;
            var anchorAt = double.PositiveInfinity;
            foreach (var s in _samples) anchorAt = Math.Min(anchorAt, s.At + (tick - s.Tick) * tickMs);
            _anchorTick = tick;
            _anchorAt = anchorAt;

            if (shown == null)
            {
                _offset = 0;
            }
            else
            {
                var raw = Raw(at);
                _offset = shown.Value - raw;
                if (Math.Abs(_offset) * tickMs > _snapMs)
                    _offset = _offset > 0 ? Math.Max(0, Math.Min(shown.Value, newestBefore) - raw) : 0;
            }
            _slewedTo = Math.Max(_slewedTo, at);
        }

        /// <summary>The estimated tick the shard is on at <paramref name="now"/>, with a fraction. Before the first frame, -1.</summary>
        public double ServerTick(double now)
        {
            if (!Ready) return -1;
            var elapsed = now - _slewedTo;
            if (elapsed > 0)
            {
                var offsetMs = Math.Abs(_offset) * TickMs;
                var rate = Math.Min(1, Math.Max(_slew, offsetMs / _snapMs));
                var step = elapsed * rate / TickMs;
                _offset -= Math.Max(-step, Math.Min(step, _offset));
                _slewedTo = now;
            }
            return Raw(now) + _offset;
        }

        double Raw(double now) => _anchorTick + (now - _anchorAt) / TickMs;

        /// <summary>The median slope over every pair of samples (Theil-Sen).</summary>
        void FitTickMs()
        {
            var n = _samples.Count;
            if (n < MinFitSamples) return;
            if (_samples[n - 1].Tick - _samples[0].Tick < 4) return;
            var slopes = new List<double>(n * (n - 1) / 2);
            for (var i = 0; i < n; i++)
            {
                for (var j = i + 1; j < n; j++)
                {
                    var dt = _samples[j].Tick - _samples[i].Tick;
                    if (dt > 0) slopes.Add((_samples[j].At - _samples[i].At) / dt);
                }
            }
            if (slopes.Count == 0) return;
            slopes.Sort();
            var slope = slopes[slopes.Count >> 1];
            if (!double.IsNaN(slope) && !double.IsInfinity(slope)) _measuredTickMs = Math.Min(2000, Math.Max(1, slope));
        }
    }
}
