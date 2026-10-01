#nullable enable
using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Net.WebSockets;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

namespace Pylon
{
    /// <summary>Settings for the live-query engine, set with <see cref="PylonClientOptions.Live"/>.</summary>
    public sealed class LiveOptions
    {
        /// <summary>The live socket URL. Default: <c>/api/sync/ws</c> on the base URL.</summary>
        public Uri? WsUrl { get; set; }

        /// <summary>
        /// Connect through the sync relay: before each connect, get a signed target from
        /// <c>GET /api/sync/relay-token</c>. For apps whose server has a relay configured.
        /// </summary>
        public bool UseRelay { get; set; }

        public TimeSpan PingInterval { get; set; } = TimeSpan.FromSeconds(25);
        public TimeSpan ReconnectBaseDelay { get; set; } = TimeSpan.FromMilliseconds(500);
        public TimeSpan ReconnectMaxDelay { get; set; } = TimeSpan.FromSeconds(30);
    }

    /// <summary>
    /// The engine behind <see cref="LiveQuery"/>: an in-memory replica of the
    /// rows the caller may read, kept current from <c>/api/sync/pull</c> and
    /// the live socket. The same rules as the TypeScript and Swift engines:
    /// <list type="bullet">
    /// <item>a pull pages through the snapshot and the change log; live frames that arrive meanwhile wait and then apply against the new cursor;</item>
    /// <item>a change applies only above the cursor, and a delete or a revocation fences older inserts and updates of its row;</item>
    /// <item>410 RESYNC_REQUIRED starts the replica over once, then backs off;</item>
    /// <item>after a reconnect it pulls from the cursor, then removes rows the server no longer returns (a revocation sent while disconnected).</item>
    /// </list>
    /// </summary>
    internal sealed class LiveSync : IDisposable
    {
        readonly PylonClient _client;
        readonly LiveOptions _options;
        readonly object _gate = new object();
        readonly Dictionary<string, Dictionary<string, PylonValue>> _tables =
            new Dictionary<string, Dictionary<string, PylonValue>>(StringComparer.Ordinal);
        readonly Dictionary<string, ulong> _tombstones = new Dictionary<string, ulong>(StringComparer.Ordinal);
        readonly List<LiveQuery> _queries = new List<LiveQuery>();
        readonly SemaphoreSlim _pullLock = new SemaphoreSlim(1, 1);
        readonly Random _random = new Random();

        ulong _cursor;
        long _revision;
        /// <summary>Bumped by every session change; a socket connected under an older one is dropped.</summary>
        int _session;
        bool _synced;
        int _epoch;
        int _consecutive410;
        List<PylonValue>? _hold;
        string? _syncToken;
        bool _tokenObserved;
        CancellationTokenSource? _run;
        /// <summary>Cancelled when the current socket's connection ends.</summary>
        CancellationTokenSource? _connection;
        ClientWebSocket? _socket;
        int _connects;

        public LiveSync(PylonClient client, LiveOptions options)
        {
            _client = client;
            _options = options;
        }

        internal ulong Cursor
        {
            get { lock (_gate) return _cursor; }
        }

        internal bool HasRow(string entity, string id)
        {
            lock (_gate) return _tables.TryGetValue(entity, out var t) && t.ContainsKey(id);
        }

        internal PylonValue? Row(string entity, string id)
        {
            lock (_gate) return _tables.TryGetValue(entity, out var t) && t.TryGetValue(id, out var r) ? r : null;
        }

        internal int Count(string entity)
        {
            lock (_gate) return _tables.TryGetValue(entity, out var t) ? t.Count : 0;
        }

        internal bool Connected
        {
            get { lock (_gate) return _socket?.State == WebSocketState.Open; }
        }

        /// <summary>
        /// The client's token changed (sign-in, sign-out, refresh to another
        /// identity). The visible set may differ: start over, and reconnect so
        /// the socket carries the new token.
        /// </summary>
        internal void OnTokenChanged()
        {
            ClientWebSocket? socket;
            lock (_gate)
            {
                if (_run == null) return;
                if (string.Equals(_client.Token, _syncToken, StringComparison.Ordinal)) return;
                ResetLocked();
                // The reconnect carries this token, and the next pull must not reset again.
                _syncToken = _client.Token;
                _session++;
                socket = _socket;
            }
            NotifyAll();
            try
            {
                socket?.Abort();
            }
            catch (Exception)
            {
                // Already closed.
            }
        }

        /// <summary>Drop the live socket without a close frame, as a lost network does.</summary>
        internal void DropSocketForTest()
        {
            ClientWebSocket? s;
            lock (_gate) s = _socket;
            try
            {
                s?.Abort();
            }
            catch (Exception)
            {
                // Already closed.
            }
        }

        // ---- queries ----

        public LiveQuery Add(string entity, LiveQueryOptions options)
        {
            var q = new LiveQuery(this, entity, options);
            bool start;
            lock (_gate)
            {
                _queries.Add(q);
                start = _run == null;
                if (start) _run = new CancellationTokenSource();
            }
            if (start)
            {
                var ct = _run!.Token;
                _ = Task.Run(() => RunAsync(ct));
            }
            else
            {
                Notify(new HashSet<string> { entity });
            }
            return q;
        }

        /// <summary>Add a query without starting the socket loop (tests drive the pull and frames).</summary>
        internal LiveQuery Attach(string entity, LiveQueryOptions options)
        {
            var q = new LiveQuery(this, entity, options);
            lock (_gate) _queries.Add(q);
            return q;
        }

        public void Remove(LiveQuery q)
        {
            CancellationTokenSource? stop = null;
            lock (_gate)
            {
                if (!_queries.Remove(q) || _queries.Count > 0) return;
                stop = _run;
                _run = null;
            }
            Stop(stop);
        }

        void Stop(CancellationTokenSource? run)
        {
            ClientWebSocket? socket;
            lock (_gate)
            {
                socket = _socket;
                _socket = null;
                ResetLocked();
            }
            run?.Cancel();
            try
            {
                socket?.Abort();
            }
            catch (Exception)
            {
                // Already closed.
            }
        }

        public void Dispose()
        {
            CancellationTokenSource? run;
            lock (_gate)
            {
                _queries.Clear();
                run = _run;
                _run = null;
            }
            Stop(run);
        }

        // ---- the connection loop ----

        async Task RunAsync(CancellationToken ct)
        {
            var attempts = 0;
            while (!ct.IsCancellationRequested)
            {
                var opened = false;
                try
                {
                    int session;
                    lock (_gate) session = _session;
                    var (url, credential) = await TargetAsync(ct).ConfigureAwait(false);
                    var socket = new ClientWebSocket();
                    if (!string.IsNullOrEmpty(credential)) socket.Options.AddSubProtocol("bearer." + Uri.EscapeDataString(credential));
                    await socket.ConnectAsync(url, ct).ConfigureAwait(false);
                    opened = true;
                    lock (_gate)
                    {
                        // The session changed while this socket connected: it carries
                        // the old token. Drop it and connect again at once.
                        if (session != _session)
                        {
                            socket.Abort();
                            socket.Dispose();
                            continue;
                        }
                        _socket = socket;
                        // Hold live frames from the first one: until the catch-up
                        // pull lands, a frame must not move the cursor past rows
                        // the pull has not delivered.
                        _hold ??= new List<PylonValue>();
                    }
                    var reconnect = Interlocked.Increment(ref _connects) > 1;
                    using var connection = CancellationTokenSource.CreateLinkedTokenSource(ct);
                    try
                    {
                        lock (_gate) _connection = connection;
                        _ = PingAsync(socket, connection.Token);
                        // Changes broadcast between the last pull and this open are
                        // in the log: pull them. After a reconnect, also drop rows the
                        // server stopped returning (a revocation sent while away).
                        _ = Task.Run(() => CatchUpAsync(reconnect, connection.Token));
                        await ReceiveAsync(socket, ct).ConfigureAwait(false);
                    }
                    finally
                    {
                        // Every exit stops this connection's ping and catch-up tasks.
                        connection.Cancel();
                        lock (_gate)
                        {
                            if (ReferenceEquals(_connection, connection)) _connection = null;
                        }
                    }
                    lock (_gate)
                    {
                        if (ReferenceEquals(_socket, socket)) _socket = null;
                    }
                    socket.Dispose();
                }
                catch (OperationCanceledException) when (ct.IsCancellationRequested)
                {
                    return;
                }
                catch (Exception e)
                {
                    _client.Dispatcher.Post(() => _client.RaiseLiveError(e));
                }
                if (ct.IsCancellationRequested) return;
                attempts = opened ? 1 : attempts + 1;
                var max = Math.Min(_options.ReconnectMaxDelay.TotalMilliseconds,
                    _options.ReconnectBaseDelay.TotalMilliseconds * Math.Pow(2, Math.Min(attempts - 1, 20)));
                try
                {
                    await Task.Delay(TimeSpan.FromMilliseconds(_random.NextDouble() * max), ct).ConfigureAwait(false);
                }
                catch (OperationCanceledException)
                {
                    return;
                }
                // Catch up before the next connect attempt.
                await PullSafeAsync(ct).ConfigureAwait(false);
            }
        }

        /// <summary>Pull until one succeeds (backing off between tries), then reconcile after a reconnect.</summary>
        async Task CatchUpAsync(bool reconnect, CancellationToken ct)
        {
            for (var attempt = 0; !ct.IsCancellationRequested; attempt++)
            {
                if (await PullSafeAsync(ct).ConfigureAwait(false))
                {
                    if (reconnect) await ReconcileAsync(ct).ConfigureAwait(false);
                    return;
                }
                var delay = Math.Min(30_000, _options.ReconnectBaseDelay.TotalMilliseconds * Math.Pow(2, Math.Min(attempt, 10)));
                try
                {
                    await Task.Delay(TimeSpan.FromMilliseconds(delay), ct).ConfigureAwait(false);
                }
                catch (OperationCanceledException)
                {
                    return;
                }
            }
        }

        async Task<(Uri Url, string? Credential)> TargetAsync(CancellationToken ct)
        {
            var token = _client.Token;
            if (_options.UseRelay)
            {
                var relay = await _client.RequestAsync("GET", "/api/sync/relay-token", null, ct).ConfigureAwait(false);
                var url = relay["url"].AsStringOr(null);
                var blob = relay["token"].AsStringOr(null);
                if (url == null || blob == null) throw PylonException.Decoding("the relay token response has no url or token");
                var sep = url.Contains("?") ? "&" : "?";
                return (new Uri($"{url}{sep}since={Cursor.ToString(CultureInfo.InvariantCulture)}"), blob);
            }
            if (_options.WsUrl != null) return (_options.WsUrl, token);
            var b = new UriBuilder(_client.BaseUrl) { Scheme = _client.BaseUrl.Scheme == "https" ? "wss" : "ws", Path = "/api/sync/ws", Query = "" };
            if (_client.BaseUrl.IsDefaultPort) b.Port = -1;
            return (b.Uri, token);
        }

        async Task PingAsync(ClientWebSocket socket, CancellationToken ct)
        {
            var ping = Encoding.UTF8.GetBytes("{\"type\":\"ping\"}");
            try
            {
                while (!ct.IsCancellationRequested)
                {
                    await Task.Delay(_options.PingInterval, ct).ConfigureAwait(false);
                    await socket.SendAsync(new ArraySegment<byte>(ping), WebSocketMessageType.Text, true, ct).ConfigureAwait(false);
                }
            }
            catch (Exception)
            {
                // The socket closed; the receive side reconnects.
            }
        }

        async Task ReceiveAsync(ClientWebSocket socket, CancellationToken ct)
        {
            var buffer = new byte[64 * 1024];
            var message = new MemoryStream();
            while (!ct.IsCancellationRequested)
            {
                WebSocketReceiveResult r;
                try
                {
                    r = await socket.ReceiveAsync(new ArraySegment<byte>(buffer), ct).ConfigureAwait(false);
                }
                catch (WebSocketException)
                {
                    return;
                }
                if (r.MessageType == WebSocketMessageType.Close) return;
                message.Write(buffer, 0, r.Count);
                if (message.Length > 64 * 1024 * 1024) return;
                if (!r.EndOfMessage) continue;
                // Binary frames carry CRDT updates, which live queries do not use.
                if (r.MessageType == WebSocketMessageType.Text) OnText(Encoding.UTF8.GetString(message.GetBuffer(), 0, (int)message.Length));
                message.SetLength(0);
            }
        }

        /// <summary>One text frame from the live socket.</summary>
        internal void OnText(string text)
        {
            PylonValue msg;
            try
            {
                msg = PylonValue.Parse(text);
            }
            catch (PylonException)
            {
                return;
            }
            // A change event: seq, entity, and kind.
            var seq = msg["seq"];
            if (seq.IsNumber && seq.AsDouble() > 0 && msg["entity"].Kind == PylonValueKind.String &&
                msg["kind"].Kind == PylonValueKind.String)
            {
                var touched = new HashSet<string>(StringComparer.Ordinal);
                lock (_gate)
                {
                    if (_hold != null)
                    {
                        _hold.Add(msg);
                        return;
                    }
                    ApplyLiveLocked(new[] { msg }, touched);
                }
                Notify(touched);
                return;
            }
            switch (msg["type"].AsStringOr(null))
            {
                case "row-revoked":
                {
                    var entity = msg["entity"].AsStringOr(null);
                    var id = msg["row_id"].AsStringOr(null);
                    if (entity == null || id == null) return;
                    lock (_gate)
                    {
                        var at = msg["seq"].IsNumber && msg["seq"].AsDouble() > 0 ? msg["seq"].AsULong() : _cursor;
                        RemoveLocked(entity, id, at);
                    }
                    Notify(new HashSet<string> { entity });
                    return;
                }
                case "session-changed":
                {
                    // The session changed server-side (an org switch, a revoke): the
                    // visible set may differ. Start over under the current token,
                    // retrying the pull for as long as this connection lasts.
                    CancellationToken ct;
                    lock (_gate)
                    {
                        ResetLocked();
                        _hold ??= new List<PylonValue>();
                        ct = _connection?.Token ?? CancellationToken.None;
                    }
                    NotifyAll();
                    _ = Task.Run(() => CatchUpAsync(false, ct));
                    return;
                }
            }
        }

        // ---- pull ----

        async Task<bool> PullSafeAsync(CancellationToken ct)
        {
            try
            {
                await PullAsync(ct).ConfigureAwait(false);
                return true;
            }
            catch (OperationCanceledException)
            {
                return false;
            }
            catch (Exception e)
            {
                _client.Dispatcher.Post(() => _client.RaiseLiveError(e));
                return false;
            }
        }

        /// <summary>Catch up from the cursor: the snapshot (from 0) and the change log, all pages.</summary>
        internal async Task PullAsync(CancellationToken ct = default)
        {
            await _pullLock.WaitAsync(ct).ConfigureAwait(false);
            try
            {
                await PullLockedAsync(ct).ConfigureAwait(false);
            }
            finally
            {
                _pullLock.Release();
            }
        }

        async Task PullLockedAsync(CancellationToken ct)
        {
            // Another identity sees other rows: start over under the new token.
            var token = _client.Token;
            var reset = false;
            lock (_gate)
            {
                if (_tokenObserved && !string.Equals(token, _syncToken, StringComparison.Ordinal))
                {
                    ResetLocked();
                    reset = true;
                }
                _syncToken = token;
                _tokenObserved = true;
            }
            if (reset)
            {
                NotifyAll();
                // The socket carries the old token: reconnect with the new one.
                ClientWebSocket? old;
                lock (_gate) old = _socket;
                try
                {
                    old?.Abort();
                }
                catch (Exception)
                {
                    // Already closed.
                }
            }

            ulong since;
            int epoch;
            lock (_gate)
            {
                since = _cursor;
                epoch = _epoch;
                // Keep a hold the socket opened: its frames wait for this pull.
                _hold ??= new List<PylonValue>();
            }
            var startedFromZero = since == 0;
            var touched = new HashSet<string>(StringComparer.Ordinal);
            try
            {
                string? snapshotAfter = null;
                var hasMore = false;
                var first = true;
                while (first || snapshotAfter != null || hasMore)
                {
                    first = false;
                    // snapshot_after is opaque and already URL-encoded by the server: append it as is.
                    var path = "/api/sync/pull?since=" + since.ToString(CultureInfo.InvariantCulture);
                    if (snapshotAfter != null) path += "&snapshot_after=" + snapshotAfter;
                    var resp = await _client.RequestAsync("GET", path, null, ct).ConfigureAwait(false);
                    var next = resp["cursor"]["last_seq"].IsNumber ? resp["cursor"]["last_seq"].AsULong() : since;
                    lock (_gate)
                    {
                        if (epoch != _epoch) return;
                        foreach (var c in resp["changes"].Items)
                        {
                            var s = c["seq"];
                            if (s.IsNumber && s.AsULong() > _cursor) ApplyLocked(c, touched);
                        }
                        if (next > _cursor) _cursor = next;
                    }
                    var advanced = next > since;
                    since = next;
                    snapshotAfter = resp["snapshot_after"].AsStringOr(null);
                    if (snapshotAfter == "") snapshotAfter = null;
                    hasMore = snapshotAfter == null && resp["has_more"].Kind == PylonValueKind.Bool &&
                              resp["has_more"].AsBool() && advanced;
                }
                List<PylonValue>? held;
                lock (_gate)
                {
                    held = _hold;
                    _hold = null;
                    if (held != null && held.Count > 0) ApplyLiveLocked(held, touched);
                    _synced = true;
                    if (!startedFromZero) _consecutive410 = 0;
                }
                NotifyAll();
            }
            catch (PylonException e) when (e.Kind == PylonErrorKind.Http && e.Status == 410)
            {
                int attempt;
                lock (_gate) attempt = _consecutive410++;
                if (attempt == 0)
                {
                    // The cursor is from another server lifetime or fell off the
                    // log: snapshot from zero, once.
                    lock (_gate) ResetLocked();
                    NotifyAll();
                    await PullLockedAsync(ct).ConfigureAwait(false);
                    return;
                }
                // Snapshotting did not converge: back off instead of re-snapshotting.
                var delay = TimeSpan.FromMilliseconds(Math.Min(30_000, 1000 * Math.Pow(2, Math.Min(attempt, 5))));
                _ = Task.Run(async () =>
                {
                    try
                    {
                        await Task.Delay(delay, ct).ConfigureAwait(false);
                        await PullSafeAsync(ct).ConfigureAwait(false);
                    }
                    catch (OperationCanceledException)
                    {
                    }
                });
            }
            finally
            {
                lock (_gate)
                {
                    // A failed pull keeps holding (frames still must not move the
                    // cursor) but drops what it held: the retry fetches those
                    // changes from the log.
                    if (_hold != null && _hold.Count > 0) _hold = new List<PylonValue>();
                }
                if (touched.Count > 0) Notify(touched);
            }
        }

        internal Task ReconcileForTestAsync() => ReconcileAsync(CancellationToken.None);

        /// <summary>Remove local rows of live entities that the server no longer returns.</summary>
        async Task ReconcileAsync(CancellationToken ct)
        {
            List<string> entities;
            lock (_gate)
            {
                entities = new List<string>();
                foreach (var q in _queries)
                {
                    if (!entities.Contains(q.Entity)) entities.Add(q.Entity);
                }
            }
            foreach (var entity in entities)
            {
                for (var tries = 0; tries < 5 && !ct.IsCancellationRequested; tries++)
                {
                    if (await ReconcileEntityAsync(entity, ct).ConfigureAwait(false)) break;
                    // A change landed during the fetch: the fetched set is stale. Fetch again.
                    await Task.Delay(TimeSpan.FromMilliseconds(100 * (tries + 1)), ct).ConfigureAwait(false);
                }
            }
        }

        /// <summary>One entity's sweep. False when a change landed during the fetch (try again).</summary>
        async Task<bool> ReconcileEntityAsync(string entity, CancellationToken ct)
        {
            {
                ulong cursorBefore;
                int epoch;
                lock (_gate)
                {
                    cursorBefore = _cursor;
                    epoch = _epoch;
                }
                var server = new HashSet<string>(StringComparer.Ordinal);
                var dropAll = false;
                try
                {
                    string? after = null;
                    var pages = 0;
                    do
                    {
                        var page = await _client.ListCursorAsync(entity, after, 500, ct, replication: true).ConfigureAwait(false);
                        foreach (var row in page.Data)
                        {
                            var id = row["id"].AsStringOr(null);
                            if (id != null) server.Add(id);
                        }
                        after = page.HasMore ? page.NextCursor : null;
                        // A table this large is out of scope for a sweep; keep what is there.
                        if (++pages > 200) return true;
                    } while (after != null);
                }
                catch (PylonException e) when (e.Kind == PylonErrorKind.Http && (e.Status == 404 || e.Status == 403))
                {
                    dropAll = true;
                }
                catch (Exception e) when (!(e is OperationCanceledException))
                {
                    _client.Dispatcher.Post(() => _client.RaiseLiveError(e));
                    return true;
                }
                var touched = false;
                lock (_gate)
                {
                    if (epoch != _epoch) return true;
                    if (_cursor != cursorBefore) return false;
                    if (!_tables.TryGetValue(entity, out var table)) return true;
                    foreach (var id in new List<string>(table.Keys))
                    {
                        if (dropAll || !server.Contains(id))
                        {
                            RemoveLocked(entity, id, cursorBefore);
                            touched = true;
                        }
                    }
                }
                if (touched) Notify(new HashSet<string> { entity });
                return true;
            }
        }

        // ---- the replica (call with _gate held) ----

        void ApplyLiveLocked(IEnumerable<PylonValue> changes, HashSet<string> touched)
        {
            foreach (var c in changes)
            {
                var seq = c["seq"].AsULong();
                if (seq <= _cursor) continue;
                ApplyLocked(c, touched);
                _cursor = seq;
            }
        }

        void ApplyLocked(PylonValue change, HashSet<string> touched)
        {
            var entity = change["entity"].AsStringOr(null);
            var id = change["row_id"].AsStringOr(null);
            var kind = change["kind"].AsStringOr(null);
            var seq = change["seq"].IsNumber ? change["seq"].AsULong() : 0;
            if (entity == null || id == null || kind == null) return;
            var key = entity + "\u0000" + id;
            if ((kind == "insert" || kind == "update") && _tombstones.TryGetValue(key, out var fence) && seq < fence) return;
            if (!_tables.TryGetValue(entity, out var table))
            {
                table = new Dictionary<string, PylonValue>(StringComparer.Ordinal);
                _tables[entity] = table;
            }
            var data = change["data"];
            switch (kind)
            {
                case "insert":
                    if (data.Kind != PylonValueKind.Object) return;
                    table[id] = WithId(data, id, null);
                    break;
                case "update":
                    if (data.Kind != PylonValueKind.Object) return;
                    table.TryGetValue(id, out var existing);
                    table[id] = WithId(data, id, existing);
                    break;
                case "delete":
                    table.Remove(id);
                    Fence(key, seq);
                    break;
                default:
                    return;
            }
            touched.Add(entity);
        }

        void RemoveLocked(string entity, string id, ulong seq)
        {
            if (_tables.TryGetValue(entity, out var table)) table.Remove(id);
            Fence(entity + "\u0000" + id, seq);
        }

        void Fence(string key, ulong seq)
        {
            if (!_tombstones.TryGetValue(key, out var old) || seq > old) _tombstones[key] = seq;
        }

        /// <summary>`data` over `existing`, with `id` set to the row id (data cannot change it).</summary>
        static PylonValue WithId(PylonValue data, string id, PylonValue? existing)
        {
            var fields = new List<KeyValuePair<string, PylonValue>>();
            if (existing != null) fields.AddRange(existing.Fields);
            fields.AddRange(data.Fields);
            fields.Add(new KeyValuePair<string, PylonValue>("id", id));
            return PylonValue.Object(fields);
        }

        void ResetLocked()
        {
            _tables.Clear();
            _tombstones.Clear();
            _cursor = 0;
            _synced = false;
            // An active hold stays (empty): the pull after a reset owns it.
            if (_hold != null) _hold = new List<PylonValue>();
            _epoch++;
        }

        // ---- delivery ----

        void NotifyAll()
        {
            HashSet<string> all;
            lock (_gate)
            {
                all = new HashSet<string>(StringComparer.Ordinal);
                foreach (var q in _queries) all.Add(q.Entity);
            }
            Notify(all);
        }

        void Notify(HashSet<string> entities)
        {
            if (entities.Count == 0) return;
            var deliveries = new List<(LiveQuery Query, List<PylonValue> Rows, bool Synced, long Revision)>();
            lock (_gate)
            {
                foreach (var q in _queries)
                {
                    if (!entities.Contains(q.Entity)) continue;
                    var rows = _tables.TryGetValue(q.Entity, out var table)
                        ? q.Select(table.Values)
                        : new List<PylonValue>();
                    // Numbered under the lock: a result taken later has a higher
                    // number, whichever thread posts first.
                    deliveries.Add((q, rows, _synced, ++_revision));
                }
            }
            foreach (var d in deliveries)
            {
                var (q, rows, synced, revision) = d;
                _client.Dispatcher.Post(() =>
                {
                    bool live;
                    lock (_gate) live = _queries.Contains(q);
                    if (live) q.Deliver(rows, synced, revision);
                });
            }
        }
    }
}
