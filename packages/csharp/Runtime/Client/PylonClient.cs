#nullable enable
using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

namespace Pylon
{
    /// <summary>Settings for <see cref="PylonClient"/>.</summary>
    public sealed class PylonClientOptions
    {
        /// <summary>The app's origin, for example <c>http://localhost:4321</c>.</summary>
        public Uri BaseUrl { get; set; }

        /// <summary>Names the token's storage key (shared with the TypeScript and Swift clients).</summary>
        public string AppName { get; set; } = "default";

        /// <summary>Headers sent with every request.</summary>
        public IDictionary<string, string> DefaultHeaders { get; } = new Dictionary<string, string>();

        /// <summary>How long one request may take. Used by the default transport.</summary>
        public TimeSpan Timeout { get; set; } = TimeSpan.FromSeconds(30);

        /// <summary>Where the session token is kept. Default: memory.</summary>
        public IPylonStorage? Storage { get; set; }

        /// <summary>How requests are sent. Default: <see cref="HttpClientTransport"/>.</summary>
        public IPylonHttpTransport? Transport { get; set; }

        /// <summary>Where session-refresh and live-query callbacks run. Default: <see cref="PylonDispatcher.Capture"/>.</summary>
        public PylonDispatcher? Dispatcher { get; set; }

        /// <summary>Settings for <see cref="PylonClient.Live"/>.</summary>
        public LiveOptions Live { get; } = new LiveOptions();

        public PylonClientOptions(Uri baseUrl)
        {
            BaseUrl = baseUrl ?? throw new ArgumentNullException(nameof(baseUrl));
        }
    }

    /// <summary>
    /// HTTP client for a Pylon app: sign-in, server functions, and entity
    /// reads and writes. Same routes and bodies as the TypeScript and Swift
    /// clients. Safe to use from any thread; every method is async and
    /// throws <see cref="PylonException"/>.
    ///
    /// Sign-in methods store the session token, and later requests send it
    /// as <c>Authorization: Bearer</c>.
    /// </summary>
    public sealed class PylonClient : IDisposable
    {
        readonly IPylonHttpTransport _transport;
        readonly bool _ownsTransport;
        readonly string _tokenKey;
        LiveSync? _live;

        public PylonClientOptions Options { get; }
        public IPylonStorage Storage { get; }
        public PylonDispatcher Dispatcher { get; }

        public PylonClient(PylonClientOptions options)
        {
            Options = options ?? throw new ArgumentNullException(nameof(options));
            Storage = options.Storage ?? new MemoryStorage();
            Dispatcher = options.Dispatcher ?? PylonDispatcher.Capture();
            if (options.Transport != null)
            {
                _transport = options.Transport;
            }
            else
            {
                _transport = new HttpClientTransport(options.Timeout);
                _ownsTransport = true;
            }
            _tokenKey = StorageKeys.Token(options.AppName);
        }

        public PylonClient(string baseUrl) : this(new PylonClientOptions(new Uri(baseUrl)))
        {
        }

        public Uri BaseUrl => Options.BaseUrl;

        // ---- session token ----

        /// <summary>The stored session token, or null.</summary>
        public string? Token => Storage.Get(_tokenKey);

        public void SetSession(string token)
        {
            if (string.IsNullOrEmpty(token)) throw PylonException.InvalidArgument("token is empty");
            Storage.Set(_tokenKey, token);
            _live?.OnTokenChanged();
        }

        public void ClearSession()
        {
            Storage.Remove(_tokenKey);
            _live?.OnTokenChanged();
        }

        // ---- auth ----

        /// <summary>Start a guest session (<c>POST /api/auth/guest</c>) and store its token.</summary>
        public async Task<SessionResponse> SignInAsGuestAsync(CancellationToken ct = default)
        {
            var session = SessionResponse.FromValue(await RequestAsync("POST", "/api/auth/guest", PylonValue.Object(), ct).ConfigureAwait(false));
            SetSession(session.Token);
            return session;
        }

        /// <summary>Email a sign-in code (<c>POST /api/auth/magic/send</c>).</summary>
        public Task StartMagicCodeAsync(string email, CancellationToken ct = default) =>
            RequestAsync("POST", "/api/auth/magic/send", PylonValue.Object(("email", email)), ct);

        /// <summary>Exchange an emailed code for a session (<c>POST /api/auth/magic/verify</c>) and store its token.</summary>
        public Task<SessionResponse> VerifyMagicCodeAsync(string email, string code, CancellationToken ct = default) =>
            SignInAsync("/api/auth/magic/verify", PylonValue.Object(("email", email), ("code", code)), ct);

        public Task<SessionResponse> SignInWithPasswordAsync(string email, string password, CancellationToken ct = default) =>
            SignInAsync("/api/auth/password/login", PylonValue.Object(("email", email), ("password", password)), ct);

        /// <summary>Create an account with a password. Registering signs in.</summary>
        public Task<SessionResponse> RegisterWithPasswordAsync(string email, string password, CancellationToken ct = default) =>
            SignInAsync("/api/auth/password/register", PylonValue.Object(("email", email), ("password", password)), ct);

        /// <summary>
        /// Sign in with Apple (<c>POST /api/auth/native/apple</c>) using the
        /// identity token from the platform's Sign in with Apple flow, and
        /// store the session. Apple gives the name to the app only on the
        /// first sign-in and never puts it in the token, so pass it then.
        /// The server must list the app's bundle id in
        /// <c>PYLON_APPLE_NATIVE_CLIENT_IDS</c>.
        /// </summary>
        public Task<SessionResponse> SignInWithAppleAsync(string idToken, string? name = null, CancellationToken ct = default)
        {
            RequireValue(idToken, nameof(idToken));
            var body = string.IsNullOrWhiteSpace(name)
                ? PylonValue.Object(("id_token", idToken))
                : PylonValue.Object(("id_token", idToken), ("name", name!.Trim()));
            return SignInAsync("/api/auth/native/apple", body, ct);
        }

        /// <summary>
        /// Sign in with Google (<c>POST /api/auth/native/google</c>) using the
        /// ID token from Google Sign-In, and store the session. The server
        /// must list the app's client id in <c>PYLON_GOOGLE_NATIVE_CLIENT_IDS</c>.
        /// </summary>
        public Task<SessionResponse> SignInWithGoogleAsync(string idToken, CancellationToken ct = default)
        {
            RequireValue(idToken, nameof(idToken));
            return SignInAsync("/api/auth/native/google", PylonValue.Object(("id_token", idToken)), ct);
        }

        /// <summary>
        /// Sign in with Steam (<c>POST /api/auth/native/steam</c>) and store the
        /// session. <paramref name="ticketHex"/> is the session ticket from
        /// <c>ISteamUser::GetAuthTicketForWebApi</c>, as hex, made with the
        /// identity the server has in <c>PYLON_STEAM_IDENTITY</c>. The package
        /// has no Steamworks dependency: get the ticket with Steamworks.NET or
        /// Facepunch.Steamworks.
        /// </summary>
        public Task<SessionResponse> SignInWithSteamAsync(string ticketHex, CancellationToken ct = default)
        {
            RequireValue(ticketHex, nameof(ticketHex));
            return SignInAsync("/api/auth/native/steam", PylonValue.Object(("ticket", ticketHex.Trim())), ct);
        }

        static void RequireValue(string value, string name)
        {
            if (string.IsNullOrWhiteSpace(value)) throw PylonException.InvalidArgument($"{name} is empty");
        }

        async Task<SessionResponse> SignInAsync(string path, PylonValue body, CancellationToken ct)
        {
            var session = SessionResponse.FromValue(await RequestAsync("POST", path, body, ct).ConfigureAwait(false));
            SetSession(session.Token);
            return session;
        }

        /// <summary>
        /// Swap the stored token for a new one with a fresh expiry
        /// (<c>POST /api/auth/refresh</c>). The old token stops working.
        /// Throws a 401 <see cref="PylonException"/> (<c>SESSION_EXPIRED</c>)
        /// when the stored token is no longer valid.
        /// </summary>
        public async Task<SessionResponse> RefreshSessionAsync(CancellationToken ct = default)
        {
            if (Token == null) throw PylonException.InvalidArgument("there is no session to refresh");
            var session = SessionResponse.FromValue(await RequestAsync("POST", "/api/auth/refresh", null, ct).ConfigureAwait(false));
            SetSession(session.Token);
            return session;
        }

        /// <summary>Who the stored token belongs to (<c>GET /api/auth/me</c>).</summary>
        public async Task<ResolvedSession> MeAsync(CancellationToken ct = default) =>
            ResolvedSession.FromValue(await RequestAsync("GET", "/api/auth/me", null, ct).ConfigureAwait(false));

        /// <summary>End the session on the server and forget the token.</summary>
        public async Task LogoutAsync(CancellationToken ct = default)
        {
            try
            {
                await RequestAsync("POST", "/api/auth/logout", null, ct).ConfigureAwait(false);
            }
            finally
            {
                ClearSession();
            }
        }

        /// <summary>
        /// Refresh the session <paramref name="margin"/> before it expires
        /// (default one hour), again after each refresh, until the handle is
        /// disposed. <paramref name="onRefresh"/> gets each new session;
        /// <paramref name="onExpired"/> runs, and the token is cleared, when
        /// the server refuses the token. A network failure retries after a
        /// minute. Callbacks run on the client's <see cref="Dispatcher"/>.
        /// </summary>
        public IDisposable StartSessionAutoRefresh(
            long expiresAt,
            Action<SessionResponse>? onRefresh = null,
            Action? onExpired = null,
            TimeSpan? margin = null)
        {
            var cts = new CancellationTokenSource();
            _ = RefreshLoopAsync(expiresAt, margin ?? TimeSpan.FromHours(1), onRefresh, onExpired, cts.Token);
            return new CancelOnDispose(cts);
        }

        async Task RefreshLoopAsync(
            long expiresAt, TimeSpan margin, Action<SessionResponse>? onRefresh, Action? onExpired, CancellationToken ct)
        {
            var next = expiresAt;
            while (!ct.IsCancellationRequested)
            {
                var now = DateTimeOffset.UtcNow.ToUnixTimeSeconds();
                var wait = TimeSpan.FromSeconds(Math.Max(0, next - now - margin.TotalSeconds));
                // Task.Delay takes at most int.MaxValue ms; a longer wait re-checks then.
                if (wait.TotalMilliseconds > int.MaxValue - 1) wait = TimeSpan.FromMilliseconds(int.MaxValue - 1);
                try
                {
                    await Task.Delay(wait, ct).ConfigureAwait(false);
                }
                catch (OperationCanceledException)
                {
                    return;
                }
                if (DateTimeOffset.UtcNow.ToUnixTimeSeconds() < next - margin.TotalSeconds) continue;
                try
                {
                    var session = await RefreshSessionAsync(ct).ConfigureAwait(false);
                    if (ct.IsCancellationRequested) return;
                    next = session.ExpiresAt ?? DateTimeOffset.UtcNow.ToUnixTimeSeconds() + (long)margin.TotalSeconds * 2;
                    if (onRefresh != null) Dispatcher.Post(() => onRefresh(session));
                }
                catch (OperationCanceledException)
                {
                    return;
                }
                catch (PylonException e) when (e.Kind == PylonErrorKind.Http && e.Status == 401 || e.Kind == PylonErrorKind.InvalidArgument)
                {
                    ClearSession();
                    if (onExpired != null) Dispatcher.Post(onExpired);
                    return;
                }
                catch (PylonException)
                {
                    try
                    {
                        await Task.Delay(TimeSpan.FromMinutes(1), ct).ConfigureAwait(false);
                    }
                    catch (OperationCanceledException)
                    {
                        return;
                    }
                    next = DateTimeOffset.UtcNow.ToUnixTimeSeconds() + (long)margin.TotalSeconds;
                }
            }
        }

        // ---- functions ----

        /// <summary>Call a server function (<c>POST /api/fn/&lt;name&gt;</c>) and return its result.</summary>
        public Task<PylonValue> CallFnAsync(string name, PylonValue? args = null, CancellationToken ct = default) =>
            RequestAsync("POST", "/api/fn/" + EscapePath(name), args ?? PylonValue.Object(), ct);

        /// <summary>Call a server function and convert its result.</summary>
        public async Task<TResult> CallFnAsync<TResult>(
            string name, PylonValue? args, IPylonConverter<TResult> result, CancellationToken ct = default) =>
            Convert(result, await CallFnAsync(name, args, ct).ConfigureAwait(false));

        /// <summary>Call a server function with converted arguments and result.</summary>
        public async Task<TResult> CallFnAsync<TArgs, TResult>(
            string name, TArgs args, IPylonConverter<TArgs> argsConverter, IPylonConverter<TResult> result,
            CancellationToken ct = default) =>
            Convert(result, await CallFnAsync(name, argsConverter.ToValue(args), ct).ConfigureAwait(false));

        // ---- entities ----

        /// <summary>Every row of an entity the caller may read (<c>GET /api/entities/&lt;entity&gt;</c>).</summary>
        public async Task<IReadOnlyList<PylonValue>> ListAsync(string entity, CancellationToken ct = default)
        {
            var body = await RequestAsync("GET", "/api/entities/" + EscapePath(entity), null, ct).ConfigureAwait(false);
            // The server wraps rows as {count, data, limit, offset}; a bare array is accepted too.
            return body.Kind == PylonValueKind.Array ? body.Items : body["data"].Items;
        }

        /// <summary>One page of rows, after the cursor <paramref name="after"/>.</summary>
        public Task<CursorPage> ListCursorAsync(
            string entity, string? after = null, int limit = 50, CancellationToken ct = default) =>
            ListCursorAsync(entity, after, limit, ct, replication: false);

        /// <param name="replication">
        /// Mark the read as a replication fetch, so the entity's <c>sync</c> scope
        /// applies (the live-query engine checking its replica).
        /// </param>
        internal async Task<CursorPage> ListCursorAsync(
            string entity, string? after, int limit, CancellationToken ct, bool replication)
        {
            var path = new StringBuilder("/api/entities/").Append(EscapePath(entity))
                .Append("/cursor?limit=").Append(limit.ToString(CultureInfo.InvariantCulture));
            if (replication) path.Append("&sync=1");
            if (!string.IsNullOrEmpty(after)) path.Append("&after=").Append(Uri.EscapeDataString(after));
            return CursorPage.FromValue(await RequestAsync("GET", path.ToString(), null, ct).ConfigureAwait(false));
        }

        /// <summary>One row by id.</summary>
        public Task<PylonValue> GetAsync(string entity, string id, CancellationToken ct = default) =>
            RequestAsync("GET", $"/api/entities/{EscapePath(entity)}/{Uri.EscapeDataString(id)}", null, ct);

        public Task<PylonValue> CreateAsync(string entity, PylonValue data, CancellationToken ct = default) =>
            RequestAsync("POST", "/api/entities/" + EscapePath(entity), data, ct);

        public Task<PylonValue> UpdateAsync(string entity, string id, PylonValue patch, CancellationToken ct = default) =>
            RequestAsync("PATCH", $"/api/entities/{EscapePath(entity)}/{Uri.EscapeDataString(id)}", patch, ct);

        public Task DeleteAsync(string entity, string id, CancellationToken ct = default) =>
            RequestAsync("DELETE", $"/api/entities/{EscapePath(entity)}/{Uri.EscapeDataString(id)}", null, ct);

        /// <summary>An aggregate query (<c>POST /api/aggregate/&lt;entity&gt;</c>): count, sum, avg, min, max, groupBy.</summary>
        public Task<PylonValue> AggregateAsync(string entity, PylonValue spec, CancellationToken ct = default) =>
            RequestAsync("POST", "/api/aggregate/" + EscapePath(entity), spec, ct);

        /// <summary>Full-text search (<c>POST /api/search/&lt;entity&gt;</c>).</summary>
        public Task<PylonValue> SearchAsync(string entity, PylonValue spec, CancellationToken ct = default) =>
            RequestAsync("POST", "/api/search/" + EscapePath(entity), spec, ct);

        // ---- live queries ----

        /// <summary>
        /// Rows of <paramref name="entity"/> the server keeps current: it pushes
        /// every insert, update, and delete the caller may see. The first call
        /// opens the live socket; disposing the last query closes it. Rows come
        /// from entities with <c>sync</c> on (the default).
        /// </summary>
        public LiveQuery Live(string entity, LiveQueryOptions? options = null)
        {
            if (string.IsNullOrEmpty(entity)) throw PylonException.InvalidArgument("entity is empty");
            LiveSync live;
            lock (_tokenKey)
            {
                live = _live ??= new LiveSync(this, Options.Live);
            }
            return live.Add(entity, options ?? new LiveQueryOptions());
        }

        /// <summary>Errors from the live-query engine (a failed pull or connect). It retries on its own.</summary>
        public event Action<Exception>? LiveError;

        internal void RaiseLiveError(Exception e) => LiveError?.Invoke(e);

        internal LiveSync? LiveEngine => _live;

        // ---- requests ----

        /// <summary>
        /// Send a request to any route and parse the JSON answer. An empty
        /// body is <see cref="PylonValue.Null"/>. A non-2xx status throws.
        /// </summary>
        public async Task<PylonValue> RequestAsync(string method, string pathAndQuery, PylonValue? body, CancellationToken ct = default)
        {
            var headers = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase) { ["Accept"] = "application/json" };
            foreach (var h in Options.DefaultHeaders) headers[h.Key] = h.Value;
            var token = Token;
            if (!string.IsNullOrEmpty(token)) headers["Authorization"] = "Bearer " + token;
            byte[]? bytes = null;
            if (body != null)
            {
                headers["Content-Type"] = "application/json";
                bytes = Encoding.UTF8.GetBytes(body.ToJson());
            }
            var url = new Uri(Options.BaseUrl, pathAndQuery);
            var response = await _transport.SendAsync(new PylonHttpRequest(method, url, headers, bytes), ct).ConfigureAwait(false);
            if (response.Status < 200 || response.Status >= 300) throw PylonException.FromResponse(response.Status, response.Body);
            if (response.Body.Length == 0) return PylonValue.Null;
            try
            {
                return PylonValue.Parse(response.Body);
            }
            catch (PylonException e)
            {
                throw PylonException.Decoding($"{method} {url.AbsolutePath}: the response is not JSON ({e.Message})", e);
            }
        }

        static T Convert<T>(IPylonConverter<T> converter, PylonValue value)
        {
            try
            {
                return converter.FromValue(value);
            }
            catch (PylonException)
            {
                throw;
            }
            catch (Exception e)
            {
                throw PylonException.Decoding($"converting the result to {typeof(T).Name} failed: {e.Message}", e);
            }
        }

        /// <summary>Percent-encode a path, keeping its slashes.</summary>
        internal static string EscapePath(string path)
        {
            if (string.IsNullOrEmpty(path)) throw PylonException.InvalidArgument("the name is empty");
            var parts = path.Split('/');
            for (var i = 0; i < parts.Length; i++) parts[i] = Uri.EscapeDataString(parts[i]);
            return string.Join("/", parts);
        }

        public void Dispose()
        {
            _live?.Dispose();
            if (_ownsTransport) ((IDisposable)_transport).Dispose();
        }

        sealed class CancelOnDispose : IDisposable
        {
            readonly CancellationTokenSource _cts;

            public CancelOnDispose(CancellationTokenSource cts)
            {
                _cts = cts;
            }

            public void Dispose()
            {
                if (!_cts.IsCancellationRequested) _cts.Cancel();
            }
        }
    }
}
