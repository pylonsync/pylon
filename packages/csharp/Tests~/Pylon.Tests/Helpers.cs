using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using Pylon;
using Xunit;

namespace Pylon.Tests
{
    static class Hex
    {
        public static byte[] Bytes(string hex)
        {
            var out_ = new byte[hex.Length / 2];
            for (var i = 0; i < out_.Length; i++) out_[i] = Convert.ToByte(hex.Substring(i * 2, 2), 16);
            return out_;
        }

        public static string Of(byte[] bytes) => string.Concat(bytes.Select(b => b.ToString("x2")));
    }

    static class Fixtures
    {
        public static PylonValue Load(string name)
        {
            var path = Path.Combine(AppContext.BaseDirectory, "fixtures", name);
            return PylonValue.Parse(File.ReadAllText(path));
        }
    }

    /// <summary>Answers requests from a queue and records them.</summary>
    sealed class MockTransport : IPylonHttpTransport
    {
        public readonly List<PylonHttpRequest> Requests = new List<PylonHttpRequest>();
        readonly Queue<Func<PylonHttpRequest, PylonHttpResponse>> _answers = new Queue<Func<PylonHttpRequest, PylonHttpResponse>>();

        public MockTransport Reply(int status, string body)
        {
            _answers.Enqueue(_ => new PylonHttpResponse(status, System.Text.Encoding.UTF8.GetBytes(body)));
            return this;
        }

        public MockTransport Fail(string message)
        {
            _answers.Enqueue(_ => throw PylonException.Transport(message));
            return this;
        }

        public Task<PylonHttpResponse> SendAsync(PylonHttpRequest request, CancellationToken cancellationToken)
        {
            lock (Requests) Requests.Add(request);
            Func<PylonHttpRequest, PylonHttpResponse> next;
            lock (_answers)
            {
                if (_answers.Count == 0) throw new InvalidOperationException($"no reply queued for {request.Method} {request.Url}");
                next = _answers.Dequeue();
            }
            return Task.FromResult(next(request));
        }

        public static string BodyOf(PylonHttpRequest r) =>
            r.Body == null ? "" : System.Text.Encoding.UTF8.GetString(r.Body);
    }

    /// <summary>
    /// A replication frame encoder for tests (the real one is in Rust).
    /// Positions are world units at a precision of 0.01.
    /// </summary>
    sealed class TestFrame
    {
        public const float Precision = 0.01f;

        public bool Full;
        public List<ulong> Despawn = new List<ulong>();
        public List<(ulong Id, double[] Pos, Dictionary<byte, byte[]?>? Components)> Spawn =
            new List<(ulong, double[], Dictionary<byte, byte[]?>?)>();
        public List<(ulong Id, double[] From, double[] To, Dictionary<byte, byte[]?>? Components)> Update =
            new List<(ulong, double[], double[], Dictionary<byte, byte[]?>?)>();

        static void Varint(List<byte> o, ulong v)
        {
            do
            {
                var b = (byte)(v & 0x7f);
                v >>= 7;
                if (v > 0) b |= 0x80;
                o.Add(b);
            } while (v > 0);
        }

        static void ZigZag(List<byte> o, long v) => Varint(o, v >= 0 ? (ulong)v << 1 : ((ulong)(-v) << 1) - 1);

        static long Q(double v) => (long)Math.Round(v / Precision);

        static void Components(List<byte> o, Dictionary<byte, byte[]?>? c)
        {
            c ??= new Dictionary<byte, byte[]?>();
            Varint(o, (ulong)c.Count);
            foreach (var kv in c)
            {
                o.Add(kv.Key);
                if (kv.Value == null) Varint(o, 0);
                else
                {
                    Varint(o, (ulong)kv.Value.Length + 1);
                    o.AddRange(kv.Value);
                }
            }
        }

        public byte[] Encode()
        {
            var o = new List<byte> { 1, (byte)(Full ? 1 : 0) };
            o.AddRange(BitConverter.GetBytes(Precision));
            Varint(o, (ulong)Despawn.Count);
            ulong? last = null;
            foreach (var id in Despawn.OrderBy(x => x))
            {
                Varint(o, last == null ? id : id - last.Value);
                last = id;
            }
            Varint(o, (ulong)Spawn.Count);
            last = null;
            foreach (var e in Spawn.OrderBy(x => x.Id))
            {
                Varint(o, last == null ? e.Id : e.Id - last.Value);
                last = e.Id;
                foreach (var p in e.Pos) ZigZag(o, Q(p));
                Components(o, e.Components);
            }
            Varint(o, (ulong)Update.Count);
            last = null;
            foreach (var e in Update.OrderBy(x => x.Id))
            {
                Varint(o, last == null ? e.Id : e.Id - last.Value);
                last = e.Id;
                var d = new long[3];
                byte mask = 0;
                for (var i = 0; i < 3; i++)
                {
                    d[i] = Q(e.To[i]) - Q(e.From[i]);
                    if (d[i] != 0) mask |= (byte)(1 << i);
                }
                if (e.Components != null) mask |= 8;
                o.Add(mask);
                foreach (var v in d)
                {
                    if (v != 0) ZigZag(o, v);
                }
                if (e.Components != null) Components(o, e.Components);
            }
            return o.ToArray();
        }

        public static double[] P(double x = 0, double y = 0, double z = 0) => new[] { x, y, z };
    }

    /// <summary>A [Fact] that runs only when an environment variable is set.</summary>
    sealed class LiveFactAttribute : FactAttribute
    {
        public LiveFactAttribute(string variable)
        {
            if (string.IsNullOrEmpty(Environment.GetEnvironmentVariable(variable)))
                Skip = $"set {variable} to run it against a live server";
        }
    }
}
