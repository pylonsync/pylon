#nullable enable
using System;
using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;

namespace Pylon
{
    // Message boundary shared by shards and live queries. Transport-specific
    // framing belongs below this boundary, not in game code.
    internal sealed class SocketMessage
    {
        public byte[] Bytes = Array.Empty<byte>();
        public bool Text;
        public int? CloseCode;
        public string? CloseReason;
        public bool Closed;
    }

    internal interface IPylonSocket : IDisposable
    {
        bool IsOpen { get; }
        Task ConnectAsync(Uri url, IReadOnlyDictionary<string, string> headers, IReadOnlyList<string> protocols, CancellationToken ct);
        Task<SocketMessage> ReceiveAsync(CancellationToken ct);
        Task SendAsync(byte[] bytes, bool text, CancellationToken ct);
        Task CloseAsync(CancellationToken ct);
        void Abort();
    }

    internal sealed class SocketDeadline : IDisposable
    {
        public CancellationTokenSource Source { get; }
        public SocketDeadline(CancellationToken ct, TimeSpan duration)
        {
            Source = CancellationTokenSource.CreateLinkedTokenSource(ct);
#if UNITY_WEBGL && !UNITY_EDITOR
            _ = SocketFactory.CancelAfter(Source, duration);
#else
            Source.CancelAfter(duration);
#endif
        }
        public void Dispose() { Source.Cancel(); Source.Dispose(); }
    }

    internal static class SocketFactory
    {
#if UNITY_WEBGL && !UNITY_EDITOR
        public const bool ContinueOnContext = true;
#else
        public const bool ContinueOnContext = false;
#endif
        public static async Task WaitAsync(SemaphoreSlim signal, CancellationToken ct)
        {
#if UNITY_WEBGL && !UNITY_EDITOR
            while (!signal.Wait(0))
            {
                ct.ThrowIfCancellationRequested();
                await Task.Yield();
            }
            ct.ThrowIfCancellationRequested();
#else
            await signal.WaitAsync(ct).ConfigureAwait(false);
#endif
        }

        public static Task Delay(int milliseconds, CancellationToken ct) => Delay(TimeSpan.FromMilliseconds(milliseconds), ct);

        public static async Task Delay(TimeSpan duration, CancellationToken ct)
        {
#if UNITY_WEBGL && !UNITY_EDITOR
            var start = System.Diagnostics.Stopwatch.StartNew();
            while (start.Elapsed < duration)
            {
                ct.ThrowIfCancellationRequested();
                await Task.Yield();
            }
            ct.ThrowIfCancellationRequested();
#else
            await Task.Delay(duration, ct).ConfigureAwait(false);
#endif
        }

        public static async Task CancelAfter(CancellationTokenSource source, TimeSpan duration)
        {
            try
            {
                await Delay(duration, source.Token).ConfigureAwait(ContinueOnContext);
                source.Cancel();
            }
            catch (OperationCanceledException) { }
            catch (ObjectDisposedException) { }
        }

        public static IPylonSocket Create(int maxBytes)
        {
#if UNITY_WEBGL && !UNITY_EDITOR
            return new BrowserSocket(maxBytes);
#else
            return new NativeSocket(maxBytes);
#endif
        }

        // Web builds have no worker threads. Start async loops on Unity's thread.
        public static Task Run(Func<Task> work)
        {
#if UNITY_WEBGL && !UNITY_EDITOR
            return work();
#else
            return Task.Run(work);
#endif
        }
    }
}
