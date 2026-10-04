#if UNITY_WEBGL && !UNITY_EDITOR
#nullable enable
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

namespace Pylon
{
    internal static class BrowserBridge
    {
        [DllImport("__Internal")] internal static extern int PylonWeb_Open(string url, string protocols, int maxBytes);
        [DllImport("__Internal")] internal static extern int PylonWeb_State(int id);
        [DllImport("__Internal")] internal static extern int PylonWeb_Next(int id);
        [DllImport("__Internal")] internal static extern int PylonWeb_Read(int id, byte[] bytes);
        [DllImport("__Internal")] internal static extern int PylonWeb_Send(int id, byte[] bytes, int length, int text);
        [DllImport("__Internal")] internal static extern int PylonWeb_CloseCode(int id);
        [DllImport("__Internal")] internal static extern int PylonWeb_CloseReason(int id, byte[] bytes, int length);
        [DllImport("__Internal")] internal static extern void PylonWeb_Release(int id);
        [DllImport("__Internal")] internal static extern int PylonWeb_Fetch(string url, string method, string headers, byte[] bytes, int length);
        [DllImport("__Internal")] internal static extern int PylonWeb_Status(int id);

        internal static string Json(IEnumerable<string> values)
        {
            var items = new List<PylonValue>();
            foreach (var value in values) items.Add(PylonValue.From(value));
            return PylonValue.Array(items.ToArray()).ToJson();
        }
    }

    internal sealed class BrowserSocket : IPylonSocket
    {
        readonly int _maxBytes;
        int _id;
        bool _disposed;
        public BrowserSocket(int maxBytes) { _maxBytes = maxBytes; }
        public bool IsOpen => _id != 0 && BrowserBridge.PylonWeb_State(_id) == 1;
        public async Task ConnectAsync(Uri url, IReadOnlyDictionary<string, string> headers, IReadOnlyList<string> protocols, CancellationToken ct)
        {
            if (_disposed) throw new ObjectDisposedException(nameof(BrowserSocket));
            ct.ThrowIfCancellationRequested();
            if (headers.Count != 0) throw new NotSupportedException("Browser WebSockets require credential subprotocols");
            _id = BrowserBridge.PylonWeb_Open(url.AbsoluteUri, BrowserBridge.Json(protocols), _maxBytes);
            try
            {
                while (BrowserBridge.PylonWeb_State(_id) == 0)
                {
                    ct.ThrowIfCancellationRequested();
                    await Task.Yield();
                }
                ct.ThrowIfCancellationRequested();
                if (!IsOpen) throw new InvalidOperationException("Browser WebSocket handshake failed. Check TLS, origin, and credentials.");
            }
            catch { Abort(); throw; }
        }
        public async Task<SocketMessage> ReceiveAsync(CancellationToken ct)
        {
            while (true)
            {
                ct.ThrowIfCancellationRequested();
                if (_disposed) throw new ObjectDisposedException(nameof(BrowserSocket));
                int length = BrowserBridge.PylonWeb_Next(_id);
                if (length >= 0)
                {
                    var bytes = new byte[length];
                    return new SocketMessage { Bytes = bytes, Text = BrowserBridge.PylonWeb_Read(_id, bytes) == 1 };
                }
                if (BrowserBridge.PylonWeb_State(_id) == 2)
                {
                    var reason = new byte[512];
                    int count = BrowserBridge.PylonWeb_CloseReason(_id, reason, reason.Length);
                    return new SocketMessage { Closed = true, CloseCode = BrowserBridge.PylonWeb_CloseCode(_id), CloseReason = Encoding.UTF8.GetString(reason, 0, count) };
                }
                await Task.Yield();
            }
        }
        public async Task SendAsync(byte[] bytes, bool text, CancellationToken ct)
        {
            while (true)
            {
                ct.ThrowIfCancellationRequested();
                if (!IsOpen) throw new InvalidOperationException("Browser WebSocket is closed");
                int result = BrowserBridge.PylonWeb_Send(_id, bytes, bytes.Length, text ? 1 : 0);
                if (result == 1) return;
                if (result < 0) throw new InvalidOperationException("Browser WebSocket send failed");
                await Task.Yield();
            }
        }
        public Task CloseAsync(CancellationToken ct) { Dispose(); return Task.CompletedTask; }
        public void Abort() => Dispose();
        public void Dispose()
        {
            if (_disposed) return;
            _disposed = true;
            if (_id != 0) BrowserBridge.PylonWeb_Release(_id);
            _id = 0;
        }
    }

    internal sealed class BrowserHttpTransport : IPylonHttpTransport, IDisposable
    {
        readonly TimeSpan _timeout;
        readonly HashSet<int> _requests = new HashSet<int>();
        bool _disposed;
        public BrowserHttpTransport(TimeSpan timeout) { _timeout = timeout; }
        public async Task<PylonHttpResponse> SendAsync(PylonHttpRequest request, CancellationToken ct)
        {
            if (_disposed) throw new ObjectDisposedException(nameof(BrowserHttpTransport));
            ct.ThrowIfCancellationRequested();
            var headers = new List<(string, PylonValue)>();
            foreach (var h in request.Headers) headers.Add((h.Key, h.Value));
            var body = request.Body ?? Array.Empty<byte>();
            int id = BrowserBridge.PylonWeb_Fetch(request.Url.AbsoluteUri, request.Method, PylonValue.Object(headers.ToArray()).ToJson(), body, request.Body == null ? -1 : body.Length);
            _requests.Add(id);
            var started = DateTime.UtcNow;
            try
            {
                while (BrowserBridge.PylonWeb_State(id) == 0)
                {
                    ct.ThrowIfCancellationRequested();
                    if (_disposed) throw new ObjectDisposedException(nameof(BrowserHttpTransport));
                    if (_timeout != Timeout.InfiniteTimeSpan && DateTime.UtcNow - started >= _timeout)
                        throw PylonException.Transport($"{request.Method} {request.Url.AbsolutePath} timed out");
                    await Task.Yield();
                }
                ct.ThrowIfCancellationRequested();
                if (_disposed) throw new ObjectDisposedException(nameof(BrowserHttpTransport));
                int status = BrowserBridge.PylonWeb_Status(id);
                if (status == 0) throw PylonException.Transport($"{request.Method} {request.Url.AbsolutePath} failed. Check TLS, CORS, and the network.");
                var bytes = new byte[BrowserBridge.PylonWeb_Next(id)];
                BrowserBridge.PylonWeb_Read(id, bytes);
                return new PylonHttpResponse(status, bytes);
            }
            finally
            {
                _requests.Remove(id);
                BrowserBridge.PylonWeb_Release(id);
            }
        }
        public void Dispose()
        {
            if (_disposed) return;
            _disposed = true;
            foreach (var id in _requests) BrowserBridge.PylonWeb_Release(id);
            _requests.Clear();
        }
    }
}
#endif
