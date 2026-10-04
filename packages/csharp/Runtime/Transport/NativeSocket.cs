#if !UNITY_WEBGL || UNITY_EDITOR
#nullable enable
using System;
using System.Collections.Generic;
using System.IO;
using System.Net.WebSockets;
using System.Threading;
using System.Threading.Tasks;

namespace Pylon
{
    internal sealed class NativeSocket : IPylonSocket
    {
        readonly ClientWebSocket _socket;
        readonly int _maxBytes;
        readonly byte[] _buffer = new byte[64 * 1024];
        readonly MemoryStream _message = new MemoryStream();
        public NativeSocket(int maxBytes, ClientWebSocket? socket = null)
        {
            _maxBytes = maxBytes;
            _socket = socket ?? new ClientWebSocket();
        }
        public bool IsOpen => _socket.State == WebSocketState.Open;
        public Task ConnectAsync(Uri url, IReadOnlyDictionary<string, string> headers, IReadOnlyList<string> protocols, CancellationToken ct)
        {
            foreach (var h in headers) _socket.Options.SetRequestHeader(h.Key, h.Value);
            foreach (var p in protocols) _socket.Options.AddSubProtocol(p);
            return _socket.ConnectAsync(url, ct);
        }
        public async Task<SocketMessage> ReceiveAsync(CancellationToken ct)
        {
            var buffer = _buffer;
            var message = _message;
            message.SetLength(0);
            while (true)
            {
                WebSocketReceiveResult r;
                try { r = await _socket.ReceiveAsync(new ArraySegment<byte>(buffer), ct).ConfigureAwait(false); }
                catch (WebSocketException)
                {
                    ct.ThrowIfCancellationRequested();
                    return new SocketMessage { Closed = true, CloseCode = (int?)_socket.CloseStatus, CloseReason = _socket.CloseStatusDescription ?? "the connection dropped" };
                }
                if (r.MessageType == WebSocketMessageType.Close)
                {
                    try
                    {
                        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(2));
                        await _socket.CloseOutputAsync(WebSocketCloseStatus.NormalClosure, "", timeout.Token).ConfigureAwait(false);
                    }
                    catch (Exception) { }
                    return new SocketMessage { Closed = true, CloseCode = (int?)r.CloseStatus, CloseReason = r.CloseStatusDescription };
                }
                if (message.Length + r.Count > _maxBytes)
                {
                    Abort();
                    return new SocketMessage { Closed = true, CloseReason = $"a frame over {_maxBytes} bytes" };
                }
                message.Write(buffer, 0, r.Count);
                if (r.EndOfMessage) return new SocketMessage { Bytes = message.ToArray(), Text = r.MessageType == WebSocketMessageType.Text };
            }
        }
        public Task SendAsync(byte[] bytes, bool text, CancellationToken ct) => _socket.SendAsync(new ArraySegment<byte>(bytes), text ? WebSocketMessageType.Text : WebSocketMessageType.Binary, true, ct);
        public Task CloseAsync(CancellationToken ct) => _socket.CloseAsync(WebSocketCloseStatus.NormalClosure, "", ct);
        public void Abort() => _socket.Abort();
        public void Dispose() => _socket.Dispose();
    }
}
#endif
