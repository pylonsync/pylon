#nullable enable
using System;
using System.Collections.Generic;
#if !UNITY_WEBGL || UNITY_EDITOR
using System.Net.Http;
#endif
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

    /// <summary>Which transport a <see cref="ShardConnection"/> uses.</summary>
    public enum ShardTransport
    {
        /// <summary>A WebSocket (TCP): works everywhere.</summary>
        WebSocket,
        /// <summary>
        /// WebTransport (QUIC) through the native plugin: entity updates
        /// travel as datagrams, so one lost packet delays one update instead
        /// of every frame behind it. Needs the plugin and an app that serves
        /// WebTransport (<c>PYLON_SHARD_WEBTRANSPORT</c>).
        /// </summary>
        WebTransport,
        /// <summary>
        /// WebTransport when it opens; otherwise a WebSocket, and WebSockets
        /// from then on (no plugin, UDP blocked, an app without it, a timeout,
        /// or datagrams that stopped arriving).
        /// </summary>
        Auto,
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

        /// <summary>
        /// WebSocket, WebTransport, or Auto. Unset: WebSocket for a
        /// <see cref="ShardConnection"/>, Auto for a <see cref="ShardGame{TInput}"/>.
        /// </summary>
        public ShardTransport? Transport { get; set; }

        /// <summary>
        /// Where to read the WebTransport endpoint (its URL and certificate
        /// hashes). Default: <c>/_pylon/shard/webtransport</c> on the app's origin.
        /// </summary>
        public Uri? WebTransportInfoUrl { get; set; }

        /// <summary>How long a WebTransport session may take to open (the endpoint request, the session, its stream). Default 3 s.</summary>
        public TimeSpan WebTransportTimeout { get; set; } = TimeSpan.FromSeconds(3);

        public bool AutoReconnect { get; set; } = true;
        public TimeSpan ReconnectBaseDelay { get; set; } = TimeSpan.FromMilliseconds(500);
        public TimeSpan ReconnectMaxDelay { get; set; } = TimeSpan.FromSeconds(10);

        /// <summary>The shard's tick rate, when known; the clock measures it otherwise.</summary>
        public double? TickRate { get; set; }

        /// <summary>Where events run. Default: <see cref="PylonDispatcher.Capture"/> (Unity's main thread when created there).</summary>
        public PylonDispatcher? Dispatcher { get; set; }

        /// <summary>
        /// The clock for frame arrival and input send times, in milliseconds
        /// (monotonic). Default <see cref="ShardClock.Now"/>. Tests pass their own.
        /// </summary>
        public Func<double>? Now { get; set; }

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

    /// <summary>
    /// The table after one or more replication frames. Over WebTransport, one
    /// update covers every frame and datagram up to a whole tick.
    /// </summary>
    public sealed class ShardReplicationUpdate
    {
        /// <summary>The newest tick the table holds (it never goes back within a connection).</summary>
        public ulong Tick { get; }
        /// <summary>The highest input sequence number the shard processed as of that tick.</summary>
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

    /// <summary>Why the connection closed.</summary>
    public sealed class ShardCloseInfo
    {
        /// <summary>
        /// The close code, or null when the connection dropped without one.
        /// A WebSocket close code (1008: refused), or over WebTransport the
        /// session's application code (1: refused, 3: try again).
        /// </summary>
        public int? Code { get; }
        public string Reason { get; }
        /// <summary>True when the connection was a WebTransport session.</summary>
        public bool WebTransport { get; }

        internal ShardCloseInfo(int? code, string reason, bool webTransport = false)
        {
            Code = code;
            Reason = reason;
            WebTransport = webTransport;
        }
    }

    /// <summary>
    /// A connection to a realtime shard: decodes snapshots and replication
    /// frames, sends inputs, and reconnects with backoff. Follows the
    /// server when it moves the subscriber to another shard (a transfer
    /// frame) or the shard to another machine.
    ///
    /// Over a WebSocket by default; set <see cref="ShardConnectionOptions.Transport"/>
    /// for WebTransport through the native plugin.
    ///
    /// Events run on <see cref="ShardConnectionOptions.Dispatcher"/>; in
    /// Unity, create the connection on the main thread and they run there.
    /// Frames are applied to <see cref="Entities"/> on that same thread, so
    /// the table is safe to read from event handlers and from Update.
    ///
    /// Unity Web uses browser networking APIs.
    /// </summary>
    public sealed class ShardConnection : IDisposable
    {
        const int PolicyClose = 1008;
        /// <summary>A WebTransport session with no whole tick for this long has lost its datagrams.</summary>
        const double StallMs = 5000;
        /// <summary>Ticks kept waiting at most; more means the datagrams stopped.</summary>
        const int MaxPendingTicks = 100;

#if !UNITY_WEBGL || UNITY_EDITOR
        static readonly HttpClient InfoHttp = new HttpClient();
#endif

        /// <summary>Opens WebTransport sessions: the native plugin. Tests set their own before <see cref="Connect"/>.</summary>
#if UNITY_WEBGL && !UNITY_EDITOR
        internal IWebTransportFactory WebTransports = BrowserWebTransport.Instance;
#else
        internal IWebTransportFactory WebTransports = NativeWebTransport.Instance;
#endif

        /// <summary>Reads the WebTransport endpoint: (HTTP status, body). Tests set their own before <see cref="Connect"/>.</summary>
        internal Func<Uri, CancellationToken, Task<(int Status, string Body)>> FetchInfo = DefaultFetchInfo;

        static async Task<(int Status, string Body)> DefaultFetchInfo(Uri url, CancellationToken ct)
        {
#if UNITY_WEBGL && !UNITY_EDITOR
            using var transport = new BrowserHttpTransport(System.Threading.Timeout.InfiniteTimeSpan);
            var response = await transport.SendAsync(new PylonHttpRequest("GET", url, new Dictionary<string, string>(), null), ct);
            return (response.Status, Encoding.UTF8.GetString(response.Body));
#else
            using var response = await InfoHttp.GetAsync(url, ct).ConfigureAwait(SocketFactory.ContinueOnContext);
            var body = await response.Content.ReadAsStringAsync().ConfigureAwait(SocketFactory.ContinueOnContext);
            return ((int)response.StatusCode, body);
#endif
        }

        readonly ShardConnectionOptions _options;
        readonly PylonDispatcher _dispatcher;
        readonly object _gate = new object();
        readonly string _initialShardId;
        readonly CancellationTokenSource _cts = new CancellationTokenSource();
        readonly Random _random = new Random();
        readonly Func<double> _now;
        readonly ShardTransport _transport;

        string _shardId;
        string? _transferTicket;
        bool _transferring;
        byte? _codec;
        ulong _clientSeq;
        int _attempts;
        bool _started;
        bool _disposed;
        /// <summary>Auto after WebTransport failed to open (or stalled): WebSockets from now on.</summary>
        bool _webSocketOnly;
        ShardConnectionState _state = ShardConnectionState.Disconnected;
        Link? _link;
        ShardTransport? _linkTransport;
        readonly SortedDictionary<ulong, double> _sentAt = new SortedDictionary<ulong, double>();

        // Owned by the dispatcher thread.
        readonly EntityTable _entities = new EntityTable();
        readonly ShardClock _clock;
        ulong _lastTick;
        ulong _lastAck;
        double? _rttMs;
        // The newest tick and ack this link has seen (-1: none yet). Over
        // WebTransport, stream frames and datagrams can arrive out of order.
        long _linkTick = -1;
        ulong _linkAck;
        // The tick replication handlers last got: they see ticks in order.
        long _reportedTick = -1;
        // Over WebTransport: the newest tick the table holds whole, the
        // datagrams of newer ticks, and the stream frames that wait with them.
        long _wholeTick = -1;
        readonly SortedDictionary<ulong, PendingTick> _pending = new SortedDictionary<ulong, PendingTick>();
        readonly List<(ulong Tick, byte[] Payload)> _pendingStream = new List<(ulong, byte[])>();
        long _receivedStreamTick = -1;
        double _lastWholeAt;
        // Whether this link replicates entities (only then does the server send datagrams every tick).
        bool _replicating;

        sealed class PendingTick
        {
            public readonly ulong Parts;
            public readonly ulong StreamTick;
            public readonly ulong Ack;
            /// <summary>By datagram number (a duplicate replaces itself).</summary>
            public readonly SortedDictionary<ulong, byte[]> Datagrams = new SortedDictionary<ulong, byte[]>();

            public PendingTick(ulong parts, ulong streamTick, ulong ack)
            {
                Parts = parts;
                StreamTick = streamTick;
                Ack = ack;
            }
        }

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
            : this(shardId, options, ShardTransport.WebSocket)
        {
        }

        /// <param name="defaultTransport">The transport when <see cref="ShardConnectionOptions.Transport"/> is unset.</param>
        internal ShardConnection(string shardId, ShardConnectionOptions options, ShardTransport defaultTransport)
        {
            if (string.IsNullOrEmpty(shardId)) throw PylonException.InvalidArgument("shardId is empty");
            _options = options ?? throw new ArgumentNullException(nameof(options));
            if (options.WsUrl == null && options.BaseUrl == null)
                throw PylonException.InvalidArgument("set BaseUrl or WsUrl");
            _shardId = shardId;
            _initialShardId = shardId;
            _dispatcher = options.Dispatcher ?? PylonDispatcher.Capture();
            _clock = new ShardClock(options.TickRate);
            _now = options.Now ?? ShardClock.Now;
            _transport = options.Transport ?? defaultTransport;
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

        /// <summary>The transport of the open link, or of the last one (null before the first). Set before <see cref="State"/> becomes Connected.</summary>
        public ShardTransport? Transport
        {
            get { lock (_gate) return _linkTransport; }
        }

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
            _ = SocketFactory.Run(RunAsync);
        }

        /// <summary>
        /// Send an input as <c>{ input, client_seq }</c>: MessagePack for a
        /// MessagePack shard, JSON otherwise. Returns its sequence number,
        /// which later frames acknowledge, or 0 when nothing was sent (the
        /// connection is not open, or <see cref="ShardConnectionOptions.MaxFrameBytes"/>
        /// of inputs wait for a socket that stopped draining). Do not
        /// predict an input that returned 0.
        /// </summary>
        public ulong Send(PylonValue input) => Send(input, (double?)null);

        /// <summary>
        /// <see cref="Send(PylonValue)"/> with the tick (fractional) the client
        /// was drawing when the player acted, for a shard with lag
        /// compensation. <see cref="ShardGame{TInput}"/> sets it from the last
        /// <c>Frame</c>.
        /// </summary>
        public ulong Send(PylonValue input, double? viewTick)
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
                    ShardWire.EncodeInput(_codec, input, _clientSeq + 1, viewTick, out var text, out var binary);
                    // Numbered and queued under one lock, so inputs reach the
                    // server in sequence order: its ack is the highest
                    // sequence it processed.
                    if (link.Enqueue(text != null ? Encoding.UTF8.GetBytes(text) : binary!, text != null))
                    {
                        seq = ++_clientSeq;
                        _sentAt[seq] = _now();
                        if (_sentAt.Count > 1024)
                        {
                            using var e = _sentAt.Keys.GetEnumerator();
                            e.MoveNext();
                            _sentAt.Remove(e.Current);
                        }
                    }
                    else
                    {
                        failure = $"Cannot send: {_options.MaxFrameBytes} bytes of inputs are waiting to go out";
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
                var ticket = await NextTicketAsync(ct).ConfigureAwait(SocketFactory.ContinueOnContext);
                if (ct.IsCancellationRequested || State == ShardConnectionState.Failed) return;

                Link? link = null;
                var mode = _transport;
                bool webSocketOnly;
                lock (_gate) webSocketOnly = _webSocketOnly;
                if (mode == ShardTransport.WebTransport || (mode == ShardTransport.Auto && !webSocketOnly))
                {
                    try
                    {
                        link = await OpenWebTransportAsync(ticket, ct).ConfigureAwait(SocketFactory.ContinueOnContext);
                    }
                    catch (OperationCanceledException) when (ct.IsCancellationRequested)
                    {
                        return;
                    }
                    catch (WebTransportUnavailable e)
                    {
                        if (mode == ShardTransport.Auto)
                        {
                            // The WebSocket works where WebTransport does not: no
                            // plugin, UDP blocked, an app without it. A failed fetch of
                            // the endpoint info may pass; the rest do not.
                            if (e.Kind != WebTransportUnavailable.Reason.Transient)
                            {
                                lock (_gate) _webSocketOnly = true;
                            }
                        }
                        else
                        {
                            RaiseError(PylonException.Transport($"WebTransport to shard {ShardId}: {e.Message}", e));
                            if (e.Kind == WebTransportUnavailable.Reason.Unsupported)
                            {
                                SetState(ShardConnectionState.Failed, e.Message);
                                return;
                            }
                            if (!_options.AutoReconnect)
                            {
                                SetState(ShardConnectionState.Disconnected, e.Message);
                                return;
                            }
                            if (!await BackoffAsync(ct).ConfigureAwait(SocketFactory.ContinueOnContext)) return;
                            continue;
                        }
                    }
                }

                ShardCloseInfo close;
                WsLink? ws = null;
                try
                {
                    if (link == null)
                    {
                        var socket = SocketFactory.Create(_options.MaxFrameBytes);
                        ws = new WsLink(socket, _options.MaxFrameBytes);
                        link = ws;
                        await ConnectSocketAsync(socket, BuildUrl(), ticket, ct).ConfigureAwait(SocketFactory.ContinueOnContext);
                    }
                    Began(link);
                    close = ws != null
                        ? await ReceiveAsync(ws, ct).ConfigureAwait(SocketFactory.ContinueOnContext)
                        : await PollAsync((WtLink)link, ct).ConfigureAwait(SocketFactory.ContinueOnContext);
                }
                catch (OperationCanceledException) when (ct.IsCancellationRequested)
                {
                    link?.Dispose();
                    return;
                }
                catch (Exception e)
                {
                    RaiseError(PylonException.Transport($"shard {ShardId}: {e.Message}", e));
                    close = new ShardCloseInfo(null, e.Message, ws == null);
                }
                finally
                {
                    lock (_gate)
                    {
                        if (ReferenceEquals(_link, link)) _link = null;
                    }
                }
                link?.Dispose();
                if (ct.IsCancellationRequested) return;
                Post(() => Closed?.Invoke(close));

                bool transferring;
                bool refused;
                lock (_gate)
                {
                    transferring = _transferring;
                    _transferring = false;
                    refused = RefusedForGood(close.Code, close.Reason, _options.TicketProvider != null,
                        _transferTicket != null, close.WebTransport);
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
                if (!await BackoffAsync(ct).ConfigureAwait(SocketFactory.ContinueOnContext)) return;
            }
        }

        async Task<bool> BackoffAsync(CancellationToken ct)
        {
            TimeSpan delay;
            lock (_gate)
            {
                _attempts++;
                delay = Backoff(_attempts);
            }
            try
            {
                await SocketFactory.Delay(delay, ct).ConfigureAwait(SocketFactory.ContinueOnContext);
                return true;
            }
            catch (OperationCanceledException)
            {
                return false;
            }
        }

        /// <summary>A link opened: its frames start over.</summary>
        void Began(Link link)
        {
            lock (_gate)
            {
                _cts.Token.ThrowIfCancellationRequested();
                _link = link;
                _linkTransport = link is WtLink ? ShardTransport.WebTransport : ShardTransport.WebSocket;
                _state = ShardConnectionState.Connected;
                // Inputs sent on the old connection are never acknowledged on
                // this one (acks restart with the connection).
                _sentAt.Clear();
            }
            Post(() =>
            {
                _linkTick = -1;
                _linkAck = 0;
                _reportedTick = -1;
                _wholeTick = -1;
                _pending.Clear();
                _pendingStream.Clear();
                _receivedStreamTick = -1;
                _lastWholeAt = _now();
                _replicating = false;
                StateChanged?.Invoke(ShardConnectionState.Connected, null);
                Opened?.Invoke();
            });
        }

        async Task<ShardCloseInfo> ReceiveAsync(WsLink link, CancellationToken ct)
        {
            link.StartSending(ct, RaiseError);
            var socket = link.Socket;
            while (true)
            {
                SocketMessage result;
                using var idle = CancellationTokenSource.CreateLinkedTokenSource(ct);
                if (_options.IdleTimeout.HasValue)
                {
#if UNITY_WEBGL && !UNITY_EDITOR
                    _ = SocketFactory.CancelAfter(idle, _options.IdleTimeout.Value);
#else
                    idle.CancelAfter(_options.IdleTimeout.Value);
#endif
                }
                try { result = await socket.ReceiveAsync(idle.Token).ConfigureAwait(SocketFactory.ContinueOnContext); }
                catch (OperationCanceledException) when (!ct.IsCancellationRequested)
                {
                    link.Abort();
                    return new ShardCloseInfo(null, $"no frame for {_options.IdleTimeout!.Value.TotalSeconds:0.#} s");
                }
                finally { idle.Cancel(); }
                if (result.Closed) return new ShardCloseInfo(result.CloseCode, result.CloseReason ?? "");
                if (!result.Text) OnFrame(link, result.Bytes, _now());
            }
        }

        // ---- WebTransport ----

        sealed class WebTransportUnavailable : Exception
        {
            public enum Reason
            {
                /// <summary>No plugin on this platform, or the app does not serve WebTransport.</summary>
                Unsupported,
                /// <summary>The endpoint info could not be fetched (may pass).</summary>
                Transient,
                /// <summary>The session did not open.</summary>
                Failed,
            }

            public readonly Reason Kind;

            public WebTransportUnavailable(Reason kind, string message) : base(message)
            {
                Kind = kind;
            }
        }

        internal Uri WebTransportInfoUrl()
        {
            if (_options.WebTransportInfoUrl != null) return _options.WebTransportInfoUrl;
            const string path = "/_pylon/shard/webtransport";
            if (_options.BaseUrl == null && _options.WsUrl != null && !_options.WsPort.HasValue)
            {
                var w = _options.WsUrl;
                return new UriBuilder(w.Scheme == "wss" ? "https" : "http", w.Host, w.Port, path).Uri;
            }
            var b = new UriBuilder(_options.BaseUrl!) { Path = path, Query = "" };
            if (_options.BaseUrl!.IsDefaultPort) b.Port = -1;
            return b.Uri;
        }

        async Task<WtLink> OpenWebTransportAsync(string? ticket, CancellationToken ct)
        {
            var factory = WebTransports;
            if (!factory.Available)
                throw new WebTransportUnavailable(WebTransportUnavailable.Reason.Unsupported,
#if UNITY_WEBGL && !UNITY_EDITOR
                    "this browser has no WebTransport");
#else
                    WebTransportNative.Version() is string v
                        ? $"the WebTransport plugin is version {v}; this client needs {WebTransportNative.AbiMajor}.x"
                        : "the WebTransport plugin is not loaded on this platform");
#endif
            // One deadline for the whole open: the endpoint request, the session, and its stream.
            using var timeout = new SocketDeadline(ct, _options.WebTransportTimeout);
            var deadline = timeout.Source;
            var timeoutMs = _options.WebTransportTimeout.TotalMilliseconds;
            string url;
            byte[][] hashes;
            try
            {
                var (status, body) = await FetchInfo(WebTransportInfoUrl(), deadline.Token).ConfigureAwait(SocketFactory.ContinueOnContext);
                if (status == 404)
                    throw new WebTransportUnavailable(WebTransportUnavailable.Reason.Unsupported, "the app does not serve WebTransport");
                if (status < 200 || status > 299)
                    throw new WebTransportUnavailable(WebTransportUnavailable.Reason.Transient,
                        $"fetching the endpoint: HTTP {status}");
                (url, hashes) = ShardWebTransport.DecodeInfo(PylonValue.Parse(body));
            }
            catch (WebTransportUnavailable)
            {
                throw;
            }
            catch (OperationCanceledException) when (ct.IsCancellationRequested)
            {
                throw;
            }
            catch (OperationCanceledException)
            {
                // The deadline: an endpoint that does not answer is treated as a session that did not open.
                throw new WebTransportUnavailable(WebTransportUnavailable.Reason.Failed,
                    $"the endpoint did not answer in {timeoutMs:0} ms");
            }
            catch (Exception e)
            {
                // The app did not answer: the WebSocket may still work, and a later attempt may reach the endpoint.
                throw new WebTransportUnavailable(WebTransportUnavailable.Reason.Transient, $"fetching the endpoint: {e.Message}");
            }

            var session = factory.Connect(url, hashes) ??
                throw new WebTransportUnavailable(WebTransportUnavailable.Reason.Failed, $"the plugin refused the endpoint {url}");
            try
            {
                while (true)
                {
                    var state = session.State;
                    if (state == WebTransportNative.StateOpen) break;
                    if (state != WebTransportNative.StateConnecting)
                        throw new WebTransportUnavailable(WebTransportUnavailable.Reason.Failed,
                            $"the session did not open: {session.Error}");
                    if (deadline.IsCancellationRequested)
                    {
                        session.Close(ShardWebTransport.CloseNormal, "");
                        ct.ThrowIfCancellationRequested();
                        throw new WebTransportUnavailable(WebTransportUnavailable.Reason.Failed,
                            $"the session did not open in {timeoutMs:0} ms");
                    }
                    await SocketFactory.Delay(5, CancellationToken.None).ConfigureAwait(SocketFactory.ContinueOnContext);
                }
                string shard;
                lock (_gate) shard = _shardId;
                session.StreamWrite(ShardWebTransport.Hello(shard, _options.SubscriberId, ticket, _options.Token));
                return new WtLink(session, _options.MaxFrameBytes);
            }
            catch
            {
                session.Dispose();
                throw;
            }
        }

        /// <summary>
        /// Drain the session's stream bytes and datagrams until it ends. Polls:
        /// the plugin calls nothing back.
        /// </summary>
        async Task<ShardCloseInfo> PollAsync(WtLink link, CancellationToken ct)
        {
#if !UNITY_WEBGL || UNITY_EDITOR
            using var watchdog = new Timer(_ => Post(() =>
            {
                if (ReferenceEquals(CurrentLink, link)) Stalled(link);
            }), null, 1000, 1000);
            return await Task.Factory.StartNew(() =>
            {
#endif
                var frames = new StreamFrames(_options.MaxFrameBytes);
                var chunk = new byte[64 * 1024];
                var datagram = new byte[65536];
                var lastActivity = _now();
                while (!ct.IsCancellationRequested)
                {
                    var busy = false;
                    var state = link.Session.State;
                    while (true)
                    {
                        var n = link.Session.StreamRead(chunk);
                        if (n <= 0) break;
                        busy = true;
                        List<byte[]> complete;
                        try
                        {
                            complete = frames.Push(chunk, (int)n);
                        }
                        catch (PylonException e)
                        {
                            RaiseError(e);
                            link.Close();
                            return new ShardCloseInfo(null, e.Message, true);
                        }
                        foreach (var frame in complete) OnFrame(link, frame, _now());
                    }
                    while (true)
                    {
                        var n = link.Session.RecvDatagram(datagram);
                        if (n < 0) break;
                        busy = true;
                        var copy = new byte[n];
                        Buffer.BlockCopy(datagram, 0, copy, 0, n);
                        var at = _now();
                        Post(() => OnDatagram(link, copy, at));
                    }
                    if (busy) lastActivity = _now();
                    if (state != WebTransportNative.StateOpen && state != WebTransportNative.StateConnecting)
                    {
                        // The server's closing frame names the code and reason first;
                        // else the session's application close; else the error.
                        if (link.Notice is (uint code, string reason)) return new ShardCloseInfo((int)code, reason, true);
                        if (link.Session.CloseInfo is (uint c, string r)) return new ShardCloseInfo((int)c, r, true);
                        return new ShardCloseInfo(null, link.Session.Error, true);
                    }
                    if (_options.IdleTimeout is TimeSpan idle && _now() - lastActivity > idle.TotalMilliseconds)
                    {
                        link.Close();
                        return new ShardCloseInfo(null, $"no frame for {idle.TotalSeconds:0.#} s", true);
                    }
#if UNITY_WEBGL && !UNITY_EDITOR
                    Stalled(link);
                    await Task.Yield();
#else
                    if (!busy) Thread.Sleep(1);
#endif
                }
                return new ShardCloseInfo(null, "closed", true);
#if !UNITY_WEBGL || UNITY_EDITOR
            }, CancellationToken.None, TaskCreationOptions.LongRunning, TaskScheduler.Default).ConfigureAwait(SocketFactory.ContinueOnContext);
#endif
        }

        Link? CurrentLink
        {
            get { lock (_gate) return _link; }
        }

        /// <summary>
        /// Runs on the receive thread: the parts of a frame the connection
        /// loop needs before the link closes (a transfer, a closing frame, the
        /// codec). The rest runs on the dispatcher, in order.
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
                    _linkTick = -1;
                    _linkAck = 0;
                    if (notice.Shard != previous) Transferred?.Invoke(notice.Shard, previous);
                });
                return;
            }
            if (frame.Kind == (byte)ShardFrameKind.Closing)
            {
                // The server is about to close the session: keep why, and close it from this side.
                if (link != null)
                {
                    try
                    {
                        var v = ShardWire.DecodePayload(frame.Codec, frame.Payload);
                        link.Notice = (v["code"].IsNumber ? (uint)v["code"].AsULong() : 0u, v["reason"].AsStringOr("") ?? "");
                    }
                    catch (PylonException)
                    {
                        link.Notice = (0u, "");
                    }
                    link.Close();
                }
                return;
            }
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
            if (link != null && !ReferenceEquals(link, CurrentLink)) return;
            var kind = (ShardFrameKind)frame.Kind;
            // Only the per-tick frames time the clock: a rejection can go out
            // before its tick's frame is built.
            if (kind == ShardFrameKind.Replication || kind == ShardFrameKind.Snapshot) ObserveTick(frame.Tick, at);
            switch (kind)
            {
                case ShardFrameKind.Replication:
                {
                    _replicating = true;
                    var webTransport = link is WtLink;
                    if (webTransport && !EntityTable.IsFullFrame(frame.Payload))
                    {
                        // It waits with its tick's datagrams (see ApplyWhole).
                        _pendingStream.Add((frame.Tick, Copy(frame.Payload)));
                        _receivedStreamTick = Math.Max(_receivedStreamTick, (long)frame.Tick);
                        if (!Stalled(link!)) ApplyWhole(link!, at);
                        return;
                    }
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
                        if (link is WsLink ws) ws.Abort();
                        else link?.Close();
                        return;
                    }
                    // A full frame holds everything, so its ack describes the table.
                    TakeAck(frame.Ack, at);
                    if (webTransport)
                    {
                        // What was built before it describes a table that is gone.
                        _wholeTick = Math.Max(_wholeTick, (long)frame.Tick);
                        _lastWholeAt = at;
                        _receivedStreamTick = Math.Max(_receivedStreamTick, (long)frame.Tick);
                        var stale = new List<ulong>();
                        foreach (var t in _pending.Keys)
                        {
                            if (t <= frame.Tick) stale.Add(t);
                        }
                        foreach (var t in stale) _pending.Remove(t);
                        _pendingStream.RemoveAll(f => f.Tick <= frame.Tick);
                    }
                    ResetBackoff();
                    Report(summary, frame.Tick);
                    if (webTransport) ApplyWhole(link!, at);
                    return;
                }
                case ShardFrameKind.Snapshot:
                {
                    TakeAck(frame.Ack, at);
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
                    ResetBackoff();
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

        /// <summary>
        /// A WebTransport datagram: entity updates for one tick. It waits
        /// until its tick is whole. One for a tick at or before the newest
        /// whole tick is dropped unacked, as if lost: the server sends its
        /// changes again. Runs on the dispatcher.
        /// </summary>
        void OnDatagram(WtLink link, byte[] datagram, double at)
        {
            if (!ReferenceEquals(link, CurrentLink)) return;
            try
            {
                var h = EntityTable.ReadDatagramHeader(new ArraySegment<byte>(datagram));
                _replicating = true;
                if ((long)h.Tick <= _wholeTick || h.Parts < 1) return;
                ObserveTick(h.Tick, at);
                if (!_pending.TryGetValue(h.Tick, out var p))
                {
                    p = new PendingTick(h.Parts, h.StreamTick, h.Ack);
                    _pending[h.Tick] = p;
                }
                p.Datagrams[h.Frame] = datagram;
                if (!Stalled(link)) ApplyWhole(link, at);
            }
            catch (ReplicationException e)
            {
                // Not a datagram this client can read: start over with a baseline.
                Error?.Invoke(e);
                _entities.Clear();
                link.Close();
            }
        }

        /// <summary>
        /// Apply everything buffered up to the newest whole tick, oldest tick
        /// first (each tick's stream frames, then its datagrams), then take
        /// that tick's ack, tell the handlers once, and ack the datagrams. A
        /// tick is whole when all its datagrams are here and so are the
        /// stream frames sent by then. Until then its changes wait, so the
        /// handlers never see a table newer than the ack they get with it.
        /// </summary>
        void ApplyWhole(Link from, double at)
        {
            try
            {
                ApplyWholeTick(from, at);
            }
            catch (ReplicationException e)
            {
                // Out of sync with the server. Reconnecting gets a full baseline.
                Error?.Invoke(e);
                _entities.Clear();
                from.Close();
            }
        }

        void ApplyWholeTick(Link from, double at)
        {
            long whole = -1;
            foreach (var kv in _pending)
            {
                var p = kv.Value;
                if ((long)kv.Key > whole && (ulong)p.Datagrams.Count == p.Parts && _receivedStreamTick >= (long)p.StreamTick)
                    whole = (long)kv.Key;
            }
            if (whole < 0) return;
            var ack = _pending[(ulong)whole].Ack;
            var ticks = new SortedSet<ulong>();
            foreach (var t in _pending.Keys)
            {
                if ((long)t <= whole) ticks.Add(t);
            }
            foreach (var f in _pendingStream)
            {
                if ((long)f.Tick <= whole) ticks.Add(f.Tick);
            }
            var spawned = new HashSet<ulong>();
            var despawned = new HashSet<ulong>();
            var updated = new HashSet<ulong>();
            var acks = new List<(ulong Frame, ulong Applied)>();
            foreach (var t in ticks)
            {
                while (_pendingStream.Count > 0 && _pendingStream[0].Tick == t)
                {
                    var f = _pendingStream[0];
                    _pendingStream.RemoveAt(0);
                    var s = _entities.Apply(f.Payload, f.Tick);
                    foreach (var id in s.Despawned)
                    {
                        despawned.Add(id);
                        spawned.Remove(id);
                        updated.Remove(id);
                    }
                    foreach (var id in s.Spawned) spawned.Add(id);
                    foreach (var id in s.Updated) updated.Add(id);
                }
                if (!_pending.TryGetValue(t, out var p)) continue;
                _pending.Remove(t);
                foreach (var d in p.Datagrams.Values)
                {
                    var s = _entities.ApplyDatagram(d);
                    foreach (var id in s.Updated) updated.Add(id);
                    acks.Add((s.Frame, _entities.StreamTick));
                }
            }
            foreach (var id in spawned) updated.Remove(id);
            _wholeTick = whole;
            _lastWholeAt = at;
            TakeAck(ack, at);
            ResetBackoff();
            var summary = new ReplicationSummary { Full = false };
            summary.Spawned.AddRange(spawned);
            summary.Updated.AddRange(updated);
            summary.Despawned.AddRange(despawned);
            Report(summary, (ulong)whole);
            if (acks.Count > 0) from.Ack(acks);
        }

        /// <summary>
        /// Too many ticks without one whole: the datagrams stopped arriving
        /// (UDP blocked partway, say). Close the session; Auto uses WebSockets
        /// from then on. Runs on the dispatcher.
        /// </summary>
        bool Stalled(Link from)
        {
            if (!ReferenceEquals(from, CurrentLink) || !_replicating ||
                (_now() - _lastWholeAt <= StallMs && _pending.Count <= MaxPendingTicks && _pendingStream.Count <= MaxPendingTicks))
                return false;
            Error?.Invoke(new InvalidOperationException($"WebTransport to shard {ShardId}: datagrams stopped arriving"));
            if (_transport == ShardTransport.Auto)
            {
                lock (_gate) _webSocketOnly = true;
            }
            from.Close();
            return true;
        }

        /// <summary>A per-tick frame or datagram for <paramref name="tick"/> arrived: the first arrival for a tick times the clock.</summary>
        void ObserveTick(ulong tick, double at)
        {
            if ((long)tick > _linkTick)
            {
                _clock.Observe(tick, at);
                _linkTick = (long)tick;
            }
            _lastTick = (ulong)_linkTick;
        }

        /// <summary>The table now holds the shard's state after the inputs up to <paramref name="ack"/>.</summary>
        void TakeAck(ulong ack, double at)
        {
            if (ack > _linkAck) _linkAck = ack;
            _lastAck = _linkAck;
            lock (_gate)
            {
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
                    if (seq > _linkAck) break;
                    var sample = at - sent;
                    _rttMs = _rttMs == null ? sample : _rttMs + (sample - _rttMs) / 8;
                    _sentAt.Remove(seq);
                }
            }
        }

        /// <summary>A frame applied: the connection works, so the next reconnect starts from the short delay again.</summary>
        void ResetBackoff()
        {
            lock (_gate) _attempts = 0;
        }

        void Report(ReplicationSummary summary, ulong tick)
        {
            _reportedTick = Math.Max(_reportedTick, (long)tick);
            Replication?.Invoke(new ShardReplicationUpdate((ulong)_reportedTick, _lastAck, summary, _entities));
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
                    return await _options.TicketProvider(shard, ct).ConfigureAwait(SocketFactory.ContinueOnContext);
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

        Task ConnectSocketAsync(IPylonSocket socket, Uri url, string? ticket, CancellationToken ct)
        {
            var headers = new Dictionary<string, string>();
            var protocols = new List<string>();
            var subprotocols = _options.Credentials == ShardCredentialTransport.Subprotocols;
#if UNITY_WEBGL && !UNITY_EDITOR
            subprotocols = true;
#endif
            var token = _options.Token;
            if (subprotocols)
            {
                if (!string.IsNullOrEmpty(token)) protocols.Add("bearer." + Uri.EscapeDataString(token));
                if (!string.IsNullOrEmpty(ticket)) protocols.Add("ticket." + Uri.EscapeDataString(ticket));
            }
            else
            {
                if (!string.IsNullOrEmpty(token)) headers["Authorization"] = "Bearer " + token;
                if (!string.IsNullOrEmpty(ticket)) headers["X-Pylon-Shard-Ticket"] = ticket;
            }
            return socket.ConnectAsync(url, headers, protocols, ct);
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
        /// True when the server refused the credentials (a WebSocket close
        /// 1008, or a WebTransport close code 1, with an "unauthorized"
        /// reason) and the next attempt would send the same ones. A shard that
        /// is not found is worth retrying: a restarted machine brings it back.
        /// </summary>
        internal static bool RefusedForGood(int? code, string? reason, bool hasTicketProvider, bool hasTransferTicket,
            bool webTransport = false)
        {
            var refusal = webTransport ? (int)ShardWebTransport.ClosePolicy : PolicyClose;
            if (code != refusal || reason == null || !reason.StartsWith("unauthorized", StringComparison.Ordinal)) return false;
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

        /// <summary>An open connection to the shard, over either transport.</summary>
        internal abstract class Link : IDisposable
        {
            /// <summary>What the server said in its closing frame before it closed the session.</summary>
            public (uint Code, string Reason)? Notice;

            /// <summary>Queue an input envelope (JSON text or the shard's codec). False when the queue is full.</summary>
            public abstract bool Enqueue(byte[] envelope, bool json);

            /// <summary>Acknowledge datagrams: (datagram number, the table's stream tick). WebTransport only.</summary>
            public virtual void Ack(List<(ulong Frame, ulong Applied)> acks)
            {
            }

            /// <summary>Close (a close frame where the transport has one).</summary>
            public abstract void Close();

            public abstract void Dispose();
        }

        /// <summary>One WebSocket and its send queue. Sends go out in order, one at a time.</summary>
        internal sealed class WsLink : Link
        {
            public readonly IPylonSocket Socket;
            readonly Queue<(byte[] Bytes, bool Text)> _queue = new Queue<(byte[], bool)>();
            readonly SemaphoreSlim _signal = new SemaphoreSlim(0);
            readonly CancellationTokenSource _stop = new CancellationTokenSource();
            readonly int _maxQueuedBytes;
            int _queuedBytes;

            public WsLink(IPylonSocket socket, int maxQueuedBytes)
            {
                Socket = socket;
                _maxQueuedBytes = maxQueuedBytes;
            }

#if !UNITY_WEBGL || UNITY_EDITOR
            public WsLink(System.Net.WebSockets.ClientWebSocket socket, int maxQueuedBytes)
                : this(new NativeSocket(maxQueuedBytes, socket), maxQueuedBytes) { }
#endif

            public override bool Enqueue(byte[] envelope, bool json)
            {
                lock (_queue)
                {
                    if (_queuedBytes + envelope.Length > _maxQueuedBytes) return false;
                    _queue.Enqueue((envelope, json));
                    _queuedBytes += envelope.Length;
                }
                _signal.Release();
                return true;
            }

            public void StartSending(CancellationToken ct, Action<Exception> onError)
            {
                var linked = CancellationTokenSource.CreateLinkedTokenSource(ct, _stop.Token);
                _ = SocketFactory.Run(async () =>
                {
                    try
                    {
                        while (true)
                        {
                            await SocketFactory.WaitAsync(_signal, linked.Token).ConfigureAwait(SocketFactory.ContinueOnContext);
                            (byte[] Bytes, bool Text) item;
                            lock (_queue)
                            {
                                item = _queue.Dequeue();
                                _queuedBytes -= item.Bytes.Length;
                            }
                            await Socket.SendAsync(
                                item.Bytes,
                                item.Text,
                                linked.Token).ConfigureAwait(SocketFactory.ContinueOnContext);
                        }
                    }
                    catch (OperationCanceledException)
                    {
                    }
                    catch (Exception e)
                    {
                        // The socket closed; the receive side reports it.
                        if (Socket.IsOpen) onError(e);
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

            public override void Close()
            {
                _ = SocketFactory.Run(async () =>
                {
                    try
                    {
                        if (Socket.IsOpen)
                        {
                            using var t = new SocketDeadline(CancellationToken.None, TimeSpan.FromSeconds(2));
                            await Socket.CloseAsync(t.Source.Token).ConfigureAwait(SocketFactory.ContinueOnContext);
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

            public override void Dispose()
            {
                if (!_stop.IsCancellationRequested) _stop.Cancel();
                Socket.Dispose();
            }
        }

        /// <summary>A WebTransport session: inputs on its stream, acks as datagrams.</summary>
        internal sealed class WtLink : Link
        {
            public readonly IWebTransportSession Session;
            readonly int _maxQueuedBytes;

            public WtLink(IWebTransportSession session, int maxQueuedBytes)
            {
                Session = session;
                _maxQueuedBytes = maxQueuedBytes;
            }

            public override bool Enqueue(byte[] envelope, bool json)
            {
                var message = ShardWebTransport.Input(envelope, json);
                // As on the WebSocket: a peer that stopped reading gets no more inputs.
                if (Session.StreamQueued + message.Length > _maxQueuedBytes) return false;
                return Session.StreamWrite(message);
            }

            public override void Ack(List<(ulong Frame, ulong Applied)> acks)
            {
                // A datagram larger than the session allows never arrives.
                var max = Session.MaxDatagramSize;
                if (max <= 0) max = 1200;
                foreach (var message in ShardWebTransport.DatagramAckBatches(acks, max)) Session.SendDatagram(message);
            }

            public override void Close() => Session.Close(ShardWebTransport.CloseNormal, "");

            public override void Dispose() => Session.Dispose();
        }
    }
}
