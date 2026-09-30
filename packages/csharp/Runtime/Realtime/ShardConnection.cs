#nullable enable
using System;
using System.Collections.Generic;
using System.IO;
using System.Net.WebSockets;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

namespace Pylon.Realtime
{
    /// <summary>How the connection sends its credentials in the WebSocket handshake.</summary>
    public enum ShardCredentialTransport
    {
        /// <summary><c>Authorization: Bearer</c> and <c>X-Pylon-Shard-Ticket</c> headers (native platforms).</summary>
        Headers,
        /// <summary><c>bearer.&lt;token&gt;</c> and <c>ticket.&lt;ticket&gt;</c> subprotocols, for hosts that cannot set headers.</summary>
        Subprotocols,
    }

    public enum ShardConnectionState
    {
        Disconnected,
        Connecting,
        Connected,
        /// <summary>Stopped for good: the server refused the credentials and nothing can bring new ones.</summary>
        Failed,
    }

    /// <summary>Settings for <see cref="ShardConnection"/>.</summary>
    public sealed class ShardConnectionOptions
    {
        /// <summary>The app's origin (<c>http://localhost:4321</c>). The connection goes to <c>/shard</c> on it.</summary>
        public Uri? BaseUrl { get; set; }

        /// <summary>Connect to the dedicated shard port (the HTTP port + 3) instead of <c>/shard</c> on the main port.</summary>
        public int? WsPort { get; set; }

        /// <summary>The full WebSocket URL, instead of <see cref="BaseUrl"/> and <see cref="WsPort"/>.</summary>
        public Uri? WsUrl { get; set; }

        /// <summary>The subscriber id the shard knows this player by (the ticket names it).</summary>
        public string SubscriberId { get; set; } = "";

        /// <summary>A session token, when the shard authorizes by session instead of a ticket.</summary>
        public string? Token { get; set; }

        /// <summary>A shard ticket from a server function (<c>ctx.shards.ticket(...)</c>).</summary>
        public string? Ticket { get; set; }

        /// <summary>
        /// Gets a new ticket for each connection attempt, for the shard the
        /// connection goes to (it changes after a transfer). Used instead of
        /// <see cref="Ticket"/> when set. Without it, a refused or expired
        /// ticket stops the connection.
        /// </summary>
        public Func<string, CancellationToken, Task<string>>? TicketProvider { get; set; }

        public ShardCredentialTransport Credentials { get; set; } = ShardCredentialTransport.Headers;

        public bool AutoReconnect { get; set; } = true;
        public TimeSpan ReconnectBaseDelay { get; set; } = TimeSpan.FromMilliseconds(500);
        public TimeSpan ReconnectMaxDelay { get; set; } = TimeSpan.FromSeconds(10);

        /// <summary>The shard's tick rate, when known; the clock measures it otherwise.</summary>
        public double? TickRate { get; set; }

        /// <summary>Where events run. Default: <see cref="PylonDispatcher.Capture"/> (Unity's main thread when created there).</summary>
        public PylonDispatcher? Dispatcher { get; set; }

        /// <summary>A frame larger than this closes the connection.</summary>
        public int MaxFrameBytes { get; set; } = 64 * 1024 * 1024;

        /// <summary>
        /// Close and reconnect when no frame arrives for this long. Off by
        /// default. A tick-driven shard sends a frame every tick, so a few
        /// seconds finds a connection that died without closing well before
        /// TCP does. Do not set it for an event-driven shard that can stay
        /// quiet.
        /// </summary>
        public TimeSpan? IdleTimeout { get; set; }
    }

    /// <summary>One snapshot frame.</summary>
    public sealed class ShardSnapshot
    {
        public ulong Tick { get; }
        /// <summary>
        /// The highest <see cref="ShardConnection.Send"/> sequence number the
        /// shard has processed (applied or refused) as of this snapshot.
        /// </summary>
        public ulong Ack { get; }
        public byte Codec { get; }
        /// <summary>The payload bytes, for a codec other than JSON or MessagePack.</summary>
        public byte[] Payload { get; }
        /// <summary>The decoded state for JSON and MessagePack shards; null for other codecs.</summary>
        public PylonValue? State { get; }

        internal ShardSnapshot(ulong tick, ulong ack, byte codec, byte[] payload, PylonValue? state)
        {
            Tick = tick;
            Ack = ack;
            Codec = codec;
            Payload = payload;
            State = state;
        }
    }

    /// <summary>One applied replication frame.</summary>
    public sealed class ShardReplicationUpdate
    {
        public ulong Tick { get; }
        public ulong Ack { get; }
        public ReplicationSummary Summary { get; }
        /// <summary>The table after the frame. Read it in the handler; it changes with the next frame.</summary>
        public EntityTable Entities { get; }

        internal ShardReplicationUpdate(ulong tick, ulong ack, ReplicationSummary summary, EntityTable entities)
        {
            Tick = tick;
            Ack = ack;
            Summary = summary;
            Entities = entities;
        }
    }

    /// <summary>Why the WebSocket closed.</summary>
    public sealed class ShardCloseInfo
    {
        /// <summary>The close code, or null when the connection dropped without one.</summary>
        public int? Code { get; }
        public string Reason { get; }

        internal ShardCloseInfo(int? code, string reason)
        {
            Code = code;
            Reason = reason;
        }
    }

    /// <summary>
    /// A connection to a realtime shard: decodes snapshots and replication
    /// frames, sends inputs, and reconnects with backoff. Follows the
    /// server when it moves the subscriber to another shard (a transfer
    /// frame) or the shard to another machine.
    ///
    /// Events run on <see cref="ShardConnectionOptions.Dispatcher"/>; in
    /// Unity, create the connection on the main thread and they run there.
    /// Frames are applied to <see cref="Entities"/> on that same thread, so
    /// the table is safe to read from event handlers and from Update.
    ///
    /// WebGL is not supported: it has no <see cref="ClientWebSocket"/>.
    /// </summary>
    public sealed class ShardConnection : IDisposable
    {
        const int PolicyClose = 1008;

        readonly ShardConnectionOptions _options;
        readonly PylonDispatcher _dispatcher;
        readonly object _gate = new object();
        readonly string _initialShardId;
        readonly CancellationTokenSource _cts = new CancellationTokenSource();
        readonly Random _random = new Random();

        string _shardId;
        string? _transferTicket;
        bool _transferring;
        byte? _codec;
        ulong _clientSeq;
        int _attempts;
        bool _started;
        bool _disposed;
        ShardConnectionState _state = ShardConnectionState.Disconnected;
        Link? _link;
        readonly SortedDictionary<ulong, double> _sentAt = new SortedDictionary<ulong, double>();

        // Owned by the dispatcher thread.
        readonly EntityTable _entities = new EntityTable();
        readonly ShardClock _clock;
        ulong _lastTick;
        ulong _lastAck;
        double? _rttMs;

        public event Action? Opened;
        public event Action<ShardCloseInfo>? Closed;
        public event Action<ShardSnapshot>? Snapshot;
        public event Action<ShardReplicationUpdate>? Replication;
        public event Action<ShardInputRejection>? InputRejected;
        /// <summary>The server moved the subscriber: (new shard, previous shard). The connection follows on its own.</summary>
        public event Action<string, string>? Transferred;
        public event Action<ShardConnectionState, string?>? StateChanged;
        public event Action<Exception>? Error;

        public ShardConnection(string shardId, ShardConnectionOptions options)
        {
            if (string.IsNullOrEmpty(shardId)) throw PylonException.InvalidArgument("shardId is empty");
            _options = options ?? throw new ArgumentNullException(nameof(options));
            if (options.WsUrl == null && options.BaseUrl == null)
                throw PylonException.InvalidArgument("set BaseUrl or WsUrl");
            _shardId = shardId;
            _initialShardId = shardId;
            _dispatcher = options.Dispatcher ?? PylonDispatcher.Capture();
            _clock = new ShardClock(options.TickRate);
        }

        /// <summary>The shard this connection is on (or connecting to). Changes after a transfer.</summary>
        public string ShardId
        {
            get { lock (_gate) return _shardId; }
        }

        public ShardConnectionState State
        {
            get { lock (_gate) return _state; }
        }

        public bool Connected => State == ShardConnectionState.Connected;

        /// <summary>The entities a replicating shard sent. Read it on the dispatcher thread.</summary>
        public EntityTable Entities => _entities;

        /// <summary>The estimated server tick. Read it on the dispatcher thread.</summary>
        public ShardClock Clock => _clock;

        /// <summary>The tick of the newest snapshot or replication frame.</summary>
        public ulong Tick => _lastTick;

        /// <summary>The newest ack: the highest input sequence number the shard processed.</summary>
        public ulong Ack => _lastAck;

        /// <summary>A smoothed input round-trip time in milliseconds, once an input has been acknowledged.</summary>
        public double? RttMs => _rttMs;

        /// <summary>The codec inputs go in: learned from the first frame, null (JSON) until then.</summary>
        internal byte? InputCodec
        {
            get { lock (_gate) return _codec; }
        }

        /// <summary>Open the connection. It reconnects on its own until <see cref="Dispose"/>.</summary>
        public void Connect()
        {
            lock (_gate)
            {
                if (_disposed) throw new ObjectDisposedException(nameof(ShardConnection));
                if (_started) return;
                _started = true;
            }
            _ = Task.Run(RunAsync);
        }

        /// <summary>
        /// Send an input as <c>{ input, client_seq }</c>: MessagePack for a
        /// MessagePack shard, JSON otherwise. Returns its sequence number,
        /// which later frames acknowledge, or 0 when nothing was sent (the
        /// connection is not open, or <see cref="ShardConnectionOptions.MaxFrameBytes"/>
        /// of inputs wait for a socket that stopped draining). Do not
        /// predict an input that returned 0.
        /// </summary>
        public ulong Send(PylonValue input)
        {
            string? failure = null;
            ulong seq = 0;
            lock (_gate)
            {
                var link = _link;
                if (link == null || _state != ShardConnectionState.Connected)
                {
                    failure = "Cannot send: the shard connection is not open";
                }
                else
                {
                    ShardWire.EncodeInput(_codec, input, _clientSeq + 1, out var text, out var binary);
                    // Numbered and queued under one lock, so inputs reach the
                    // socket in sequence order: the server's ack is the highest
                    // sequence it processed.
                    if (link.Enqueue(text != null ? Encoding.UTF8.GetBytes(text) : binary!, text != null))
                    {
                        seq = ++_clientSeq;
                        _sentAt[seq] = ShardClock.Now();
                        if (_sentAt.Count > 1024)
                        {
                            using var e = _sentAt.Keys.GetEnumerator();
                            e.MoveNext();
                            _sentAt.Remove(e.Current);
                        }
                    }
                    else
                    {
                        failure = $"Cannot send: {_options.MaxFrameBytes} bytes of inputs are waiting for the socket";
                    }
                }
            }
            if (failure != null)
            {
                RaiseError(new InvalidOperationException(failure));
                return 0;
            }
            return seq;
        }

        /// <summary>Send an input of your own type.</summary>
        public ulong Send<T>(T input, IPylonConverter<T> converter) => Send(converter.ToValue(input));

        // ---- connection loop ----

        async Task RunAsync()
        {
            var ct = _cts.Token;
            while (!ct.IsCancellationRequested)
            {
                SetState(ShardConnectionState.Connecting, null);
                var ticket = await NextTicketAsync(ct).ConfigureAwait(false);
                if (ct.IsCancellationRequested || State == ShardConnectionState.Failed) return;

                var socket = new ClientWebSocket();
                var url = BuildUrl();
                ConfigureCredentials(socket, ticket);
                var link = new Link(socket, _options.MaxFrameBytes);
                ShardCloseInfo close;
                try
                {
                    await socket.ConnectAsync(url, ct).ConfigureAwait(false);
                    lock (_gate)
                    {
                        _link = link;
                        _state = ShardConnectionState.Connected;
                    }
                    Post(() =>
                    {
                        StateChanged?.Invoke(ShardConnectionState.Connected, null);
                        Opened?.Invoke();
                    });
                    link.StartSending(ct, RaiseError);
                    close = await ReceiveAsync(link, ct).ConfigureAwait(false);
                }
                catch (OperationCanceledException) when (ct.IsCancellationRequested)
                {
                    link.Dispose();
                    return;
                }
                catch (Exception e)
                {
                    RaiseError(PylonException.Transport($"shard {ShardId}: {e.Message}", e));
                    close = new ShardCloseInfo(
                        socket.CloseStatus.HasValue ? (int?)socket.CloseStatus.Value : null,
                        socket.CloseStatusDescription ?? "");
                }
                finally
                {
                    lock (_gate)
                    {
                        if (ReferenceEquals(_link, link)) _link = null;
                    }
                }
                link.Dispose();
                if (ct.IsCancellationRequested) return;
                Post(() => Closed?.Invoke(close));

                bool transferring;
                bool refused;
                lock (_gate)
                {
                    transferring = _transferring;
                    _transferring = false;
                    refused = RefusedForGood(close.Code, close.Reason, _options.TicketProvider != null, _transferTicket != null);
                }
                // The server closed after a transfer frame: go to the new shard at once.
                if (transferring) continue;
                if (refused)
                {
                    SetState(
                        ShardConnectionState.Failed,
                        $"shard {ShardId} refused the connection ({close.Reason}); set TicketProvider to get new tickets");
                    return;
                }
                SetState(ShardConnectionState.Disconnected, close.Reason);
                if (!_options.AutoReconnect) return;
                TimeSpan delay;
                lock (_gate)
                {
                    _attempts++;
                    delay = Backoff(_attempts);
                }
                try
                {
                    await Task.Delay(delay, ct).ConfigureAwait(false);
                }
                catch (OperationCanceledException)
                {
                    return;
                }
            }
        }

        async Task<ShardCloseInfo> ReceiveAsync(Link link, CancellationToken ct)
        {
            var socket = link.Socket;
            var buffer = new byte[64 * 1024];
            var message = new MemoryStream();
            while (true)
            {
                WebSocketReceiveResult result;
                CancellationTokenSource? idle = null;
                try
                {
                    var token = ct;
                    if (_options.IdleTimeout.HasValue)
                    {
                        idle = CancellationTokenSource.CreateLinkedTokenSource(ct);
                        idle.CancelAfter(_options.IdleTimeout.Value);
                        token = idle.Token;
                    }
                    result = await socket.ReceiveAsync(new ArraySegment<byte>(buffer), token).ConfigureAwait(false);
                }
                catch (OperationCanceledException) when (!ct.IsCancellationRequested)
                {
                    link.Abort();
                    return new ShardCloseInfo(null, $"no frame for {_options.IdleTimeout!.Value.TotalSeconds:0.#} s");
                }
                catch (WebSocketException) when (idle != null && idle.IsCancellationRequested && !ct.IsCancellationRequested)
                {
                    link.Abort();
                    return new ShardCloseInfo(null, $"no frame for {_options.IdleTimeout!.Value.TotalSeconds:0.#} s");
                }
                catch (WebSocketException)
                {
                    return new ShardCloseInfo(
                        socket.CloseStatus.HasValue ? (int?)socket.CloseStatus.Value : null,
                        socket.CloseStatusDescription ?? "the connection dropped");
                }
                finally
                {
                    idle?.Dispose();
                }
                if (result.MessageType == WebSocketMessageType.Close)
                {
                    try
                    {
                        using var closing = new CancellationTokenSource(TimeSpan.FromSeconds(2));
                        await socket.CloseOutputAsync(WebSocketCloseStatus.NormalClosure, "", closing.Token).ConfigureAwait(false);
                    }
                    catch (Exception)
                    {
                        // The server already went; nothing to acknowledge.
                    }
                    return new ShardCloseInfo(
                        result.CloseStatus.HasValue ? (int?)result.CloseStatus.Value : null,
                        result.CloseStatusDescription ?? "");
                }
                if (message.Length + result.Count > _options.MaxFrameBytes)
                {
                    link.Abort();
                    return new ShardCloseInfo(null, $"a frame over {_options.MaxFrameBytes} bytes");
                }
                message.Write(buffer, 0, result.Count);
                if (!result.EndOfMessage) continue;
                var bytes = message.ToArray();
                message.SetLength(0);
                // Text messages carry nothing in wire version 2.
                if (result.MessageType == WebSocketMessageType.Binary) OnFrame(link, bytes, ShardClock.Now());
            }
        }

        /// <summary>
        /// Runs on the receive thread: the parts of a frame the connection
        /// loop needs before the socket closes (a transfer, the codec). The
        /// rest runs on the dispatcher, in order.
        /// </summary>
        internal void OnFrame(Link? link, byte[] data, double at)
        {
            ShardFrame frame;
            try
            {
                frame = ShardWire.Parse(data);
            }
            catch (PylonException e)
            {
                RaiseError(e);
                return;
            }
            if (frame.Kind == (byte)ShardFrameKind.Transfer)
            {
                ShardTransferNotice notice;
                try
                {
                    notice = ShardWire.DecodeTransfer(frame.Codec, frame.Payload);
                }
                catch (PylonException e)
                {
                    RaiseError(e);
                    return;
                }
                string previous;
                lock (_gate)
                {
                    previous = _shardId;
                    _shardId = notice.Shard;
                    _transferTicket = notice.Ticket;
                    _codec = null;
                    _attempts = 0;
                    _transferring = true;
                    _sentAt.Clear();
                }
                // A new shard, or this one started on another machine (a
                // deploy): its ticks, acks, and entities start over. The
                // same shard on another machine is not a move for the app.
                Post(() =>
                {
                    _entities.Clear();
                    _clock.Reset();
                    _lastTick = 0;
                    _lastAck = 0;
                    if (notice.Shard != previous) Transferred?.Invoke(notice.Shard, previous);
                });
                return;
            }
            if (frame.Kind == (byte)ShardFrameKind.Closing) return;
            lock (_gate)
            {
                // The new shard answered: the provider gives the next tickets.
                if (_options.TicketProvider != null) _transferTicket = null;
                // The replication codec byte names the frame format, not the
                // shard's input codec, so only other frames set it.
                if (frame.Kind != (byte)ShardFrameKind.Replication) _codec = frame.Codec;
            }
            Post(() => Apply(link, frame, at));
        }

        /// <summary>Runs on the dispatcher.</summary>
        void Apply(Link? link, ShardFrame frame, double at)
        {
            switch ((ShardFrameKind)frame.Kind)
            {
                case ShardFrameKind.Replication:
                {
                    ReplicationSummary summary;
                    try
                    {
                        summary = _entities.Apply(frame.Payload, frame.Tick);
                    }
                    catch (ReplicationException e)
                    {
                        // Out of sync with the server. Reconnecting gets a full baseline.
                        Error?.Invoke(e);
                        _entities.Clear();
                        link?.Abort();
                        return;
                    }
                    Observe(frame, at);
                    Replication?.Invoke(new ShardReplicationUpdate(frame.Tick, frame.Ack, summary, _entities));
                    return;
                }
                case ShardFrameKind.Snapshot:
                {
                    PylonValue? state = null;
                    if (frame.Codec == (byte)ShardCodec.Json || frame.Codec == (byte)ShardCodec.MessagePack)
                    {
                        try
                        {
                            state = ShardWire.DecodePayload(frame.Codec, frame.Payload);
                        }
                        catch (PylonException e)
                        {
                            Error?.Invoke(e);
                            return;
                        }
                    }
                    Observe(frame, at);
                    Snapshot?.Invoke(new ShardSnapshot(frame.Tick, frame.Ack, frame.Codec, Copy(frame.Payload), state));
                    return;
                }
                case ShardFrameKind.InputRejected:
                {
                    ShardInputRejection rejection;
                    try
                    {
                        rejection = ShardWire.DecodeRejection(frame.Codec, frame.Payload);
                    }
                    catch (PylonException e)
                    {
                        Error?.Invoke(e);
                        return;
                    }
                    if (rejection.ClientSeq.HasValue)
                    {
                        lock (_gate) _sentAt.Remove(rejection.ClientSeq.Value);
                    }
                    InputRejected?.Invoke(rejection);
                    return;
                }
            }
        }

        /// <summary>A snapshot or replication frame applied: the tick, ack, clock, round trip, and backoff.</summary>
        void Observe(ShardFrame frame, double at)
        {
            _clock.Observe(frame.Tick, at);
            _lastTick = frame.Tick;
            if (frame.Ack > _lastAck) _lastAck = frame.Ack;
            lock (_gate)
            {
                // A frame applied: the connection works, so the next
                // reconnect starts from the short delay again.
                _attempts = 0;
                while (_sentAt.Count > 0)
                {
                    ulong seq;
                    double sent;
                    using (var e = _sentAt.GetEnumerator())
                    {
                        e.MoveNext();
                        seq = e.Current.Key;
                        sent = e.Current.Value;
                    }
                    if (seq > frame.Ack) break;
                    var sample = at - sent;
                    _rttMs = _rttMs == null ? sample : _rttMs + (sample - _rttMs) / 8;
                    _sentAt.Remove(seq);
                }
            }
        }

        static byte[] Copy(ArraySegment<byte> seg)
        {
            var out_ = new byte[seg.Count];
            Buffer.BlockCopy(seg.Array!, seg.Offset, out_, 0, seg.Count);
            return out_;
        }

        /// <summary>The ticket for the next connection: a transfer's until the new shard answers, else the provider's, else the configured one.</summary>
        async Task<string?> NextTicketAsync(CancellationToken ct)
        {
            string shard;
            string? transfer;
            lock (_gate)
            {
                shard = _shardId;
                if (_transferTicket != null && ShardTicket.Expired(_transferTicket))
                {
                    // An expired transfer ticket: a provider gives a new one;
                    // a fixed ticket still works for the shard it names.
                    _transferTicket = null;
                    if (_options.TicketProvider == null && _shardId != _initialShardId)
                    {
                        _state = ShardConnectionState.Failed;
                        var reason =
                            $"the ticket for shard {_shardId} expired before the client reconnected; set TicketProvider to get new ones";
                        Post(() => StateChanged?.Invoke(ShardConnectionState.Failed, reason));
                        return null;
                    }
                }
                transfer = _transferTicket;
            }
            if (transfer != null) return transfer;
            if (_options.TicketProvider != null)
            {
                try
                {
                    return await _options.TicketProvider(shard, ct).ConfigureAwait(false);
                }
                catch (OperationCanceledException) when (ct.IsCancellationRequested)
                {
                    return null;
                }
                catch (Exception e)
                {
                    RaiseError(e);
                    return null;
                }
            }
            return _options.Ticket;
        }

        void ConfigureCredentials(ClientWebSocket socket, string? ticket)
        {
            var token = _options.Token;
            if (_options.Credentials == ShardCredentialTransport.Subprotocols)
            {
                if (!string.IsNullOrEmpty(token)) socket.Options.AddSubProtocol("bearer." + Uri.EscapeDataString(token));
                if (!string.IsNullOrEmpty(ticket)) socket.Options.AddSubProtocol("ticket." + Uri.EscapeDataString(ticket));
                return;
            }
            if (!string.IsNullOrEmpty(token)) socket.Options.SetRequestHeader("Authorization", "Bearer " + token);
            if (!string.IsNullOrEmpty(ticket)) socket.Options.SetRequestHeader("X-Pylon-Shard-Ticket", ticket);
        }

        internal Uri BuildUrl()
        {
            string shard;
            lock (_gate) shard = _shardId;
            if (_options.WsUrl != null)
            {
                // After a transfer, the same URL with the new shard.
                var b = new UriBuilder(_options.WsUrl);
                var kept = new List<string> { "shard=" + Uri.EscapeDataString(shard) };
                foreach (var pair in b.Query.TrimStart('?').Split(new[] { '&' }, StringSplitOptions.RemoveEmptyEntries))
                {
                    if (!pair.StartsWith("shard=", StringComparison.Ordinal) && pair != "shard") kept.Add(pair);
                }
                b.Query = string.Join("&", kept);
                return b.Uri;
            }
            var ub = new UriBuilder(_options.BaseUrl!)
            {
                Scheme = _options.BaseUrl!.Scheme == "https" ? "wss" : "ws",
            };
            if (_options.WsPort.HasValue)
            {
                ub.Port = _options.WsPort.Value;
                ub.Path = "/";
            }
            else
            {
                if (_options.BaseUrl.IsDefaultPort) ub.Port = -1;
                ub.Path = "/shard";
            }
            ub.Query = "shard=" + Uri.EscapeDataString(shard) +
                       "&sid=" + Uri.EscapeDataString(_options.SubscriberId) +
                       "&v=" + ShardWire.Version;
            return ub.Uri;
        }

        /// <summary>
        /// True when the server refused the credentials (close 1008 with an
        /// "unauthorized" reason) and the next attempt would send the same
        /// ones. A shard that is not found is worth retrying: a restarted
        /// machine brings it back.
        /// </summary>
        internal static bool RefusedForGood(int? code, string? reason, bool hasTicketProvider, bool hasTransferTicket)
        {
            if (code != PolicyClose || reason == null || !reason.StartsWith("unauthorized", StringComparison.Ordinal)) return false;
            return hasTransferTicket || !hasTicketProvider;
        }

        /// <summary>Exponential backoff with full jitter, as in the Swift client.</summary>
        TimeSpan Backoff(int attempts)
        {
            var attempt = Math.Max(1, attempts);
            var max = Math.Min(
                _options.ReconnectMaxDelay.TotalMilliseconds,
                _options.ReconnectBaseDelay.TotalMilliseconds * Math.Pow(2, Math.Min(30, attempt - 1)));
            return TimeSpan.FromMilliseconds(_random.NextDouble() * max);
        }

        void SetState(ShardConnectionState state, string? reason)
        {
            lock (_gate)
            {
                if (_disposed || _state == state || _state == ShardConnectionState.Failed) return;
                _state = state;
            }
            Post(() => StateChanged?.Invoke(state, reason));
        }

        void RaiseError(Exception e) => Post(() => Error?.Invoke(e));

        void Post(Action action)
        {
            _dispatcher.Post(() =>
            {
                lock (_gate)
                {
                    if (_disposed) return;
                }
                action();
            });
        }

        /// <summary>Close the connection and stop reconnecting. No events run after this.</summary>
        public void Dispose()
        {
            Link? link;
            lock (_gate)
            {
                if (_disposed) return;
                _disposed = true;
                _state = ShardConnectionState.Disconnected;
                link = _link;
                _link = null;
            }
            _cts.Cancel();
            link?.Close();
        }

        /// <summary>One WebSocket and its send queue. Sends go out in order, one at a time.</summary>
        internal sealed class Link : IDisposable
        {
            public readonly ClientWebSocket Socket;
            readonly Queue<(byte[] Bytes, bool Text)> _queue = new Queue<(byte[], bool)>();
            readonly SemaphoreSlim _signal = new SemaphoreSlim(0);
            readonly CancellationTokenSource _stop = new CancellationTokenSource();
            readonly int _maxQueuedBytes;
            int _queuedBytes;

            public Link(ClientWebSocket socket, int maxQueuedBytes)
            {
                Socket = socket;
                _maxQueuedBytes = maxQueuedBytes;
            }

            /// <summary>False when the queue is full: the socket stopped draining.</summary>
            public bool Enqueue(byte[] bytes, bool text)
            {
                lock (_queue)
                {
                    if (_queuedBytes + bytes.Length > _maxQueuedBytes) return false;
                    _queue.Enqueue((bytes, text));
                    _queuedBytes += bytes.Length;
                }
                _signal.Release();
                return true;
            }

            public void StartSending(CancellationToken ct, Action<Exception> onError)
            {
                var linked = CancellationTokenSource.CreateLinkedTokenSource(ct, _stop.Token);
                _ = Task.Run(async () =>
                {
                    try
                    {
                        while (true)
                        {
                            await _signal.WaitAsync(linked.Token).ConfigureAwait(false);
                            (byte[] Bytes, bool Text) item;
                            lock (_queue)
                            {
                                item = _queue.Dequeue();
                                _queuedBytes -= item.Bytes.Length;
                            }
                            await Socket.SendAsync(
                                new ArraySegment<byte>(item.Bytes),
                                item.Text ? WebSocketMessageType.Text : WebSocketMessageType.Binary,
                                true,
                                linked.Token).ConfigureAwait(false);
                        }
                    }
                    catch (OperationCanceledException)
                    {
                    }
                    catch (Exception e) when (e is WebSocketException || e is ObjectDisposedException || e is InvalidOperationException)
                    {
                        // The socket closed; the receive side reports it.
                        if (Socket.State == WebSocketState.Open) onError(e);
                    }
                    finally
                    {
                        linked.Dispose();
                    }
                });
            }

            public void Abort()
            {
                try
                {
                    Socket.Abort();
                }
                catch (Exception)
                {
                    // Already closed.
                }
            }

            public void Close()
            {
                _ = Task.Run(async () =>
                {
                    try
                    {
                        if (Socket.State == WebSocketState.Open)
                        {
                            using var t = new CancellationTokenSource(TimeSpan.FromSeconds(2));
                            await Socket.CloseAsync(WebSocketCloseStatus.NormalClosure, "", t.Token).ConfigureAwait(false);
                        }
                    }
                    catch (Exception)
                    {
                        // Closing a socket that is already going.
                    }
                    finally
                    {
                        Abort();
                        Dispose();
                    }
                });
            }

            public void Dispose()
            {
                if (!_stop.IsCancellationRequested) _stop.Cancel();
                Socket.Dispose();
            }
        }
    }
}
