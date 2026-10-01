using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using Pylon;
using Xunit;

namespace Pylon.Tests
{
    /// <summary>
    /// A sync server in memory: a change log behind <c>/api/sync/pull</c>,
    /// paginated like the real one, and rows behind the cursor route.
    /// </summary>
    sealed class FakeSyncServer : IPylonHttpTransport
    {
        readonly object _gate = new object();
        readonly List<PylonValue> _log = new List<PylonValue>();
        readonly Dictionary<string, Dictionary<string, PylonValue>> _rows = new Dictionary<string, Dictionary<string, PylonValue>>();
        public ulong Seq;
        public int PageSize = 1000;
        /// <summary>Answer the next pull with this status instead.</summary>
        public int? FailNextPull;
        /// <summary>Changes below this seq are gone from the log (a since below it gets 410).</summary>
        public ulong Retained;
        /// <summary>Runs before each pull answers (to race a live frame against it).</summary>
        public Action? BeforePull;
        public int Pulls;
        public readonly HashSet<string> Hidden = new HashSet<string>();
        /// <summary>Rows only this bearer token may see (any other caller does not see them).</summary>
        public readonly Dictionary<string, string> OwnerToken = new Dictionary<string, string>();
        /// <summary>Runs before the cursor route answers.</summary>
        public Action? BeforeCursor;

        bool Visible(string id, string? token) =>
            !Hidden.Contains(id) && (!OwnerToken.TryGetValue(id, out var owner) || owner == token);

        public void Seed(string entity, string id, string kind, PylonValue? data = null)
        {
            lock (_gate)
            {
                Seq++;
                var change = PylonValue.Object(("seq", Seq), ("entity", entity), ("row_id", id), ("kind", kind),
                    ("data", data ?? PylonValue.Null), ("timestamp", ""));
                _log.Add(change);
                if (!_rows.TryGetValue(entity, out var t)) _rows[entity] = t = new Dictionary<string, PylonValue>();
                if (kind == "delete") t.Remove(id);
                else
                {
                    var fields = new List<KeyValuePair<string, PylonValue>>();
                    if (kind == "update" && t.TryGetValue(id, out var old)) fields.AddRange(old.Fields);
                    if (data != null) fields.AddRange(data.Fields);
                    fields.Add(new KeyValuePair<string, PylonValue>("id", id));
                    t[id] = PylonValue.Object(fields);
                }
            }
        }

        public Task<PylonHttpResponse> SendAsync(PylonHttpRequest request, CancellationToken cancellationToken)
        {
            var path = request.Url.AbsolutePath;
            request.Headers.TryGetValue("Authorization", out var auth);
            var token = auth?.StartsWith("Bearer ") == true ? auth.Substring(7) : null;
            var query = request.Url.Query.TrimStart('?').Split('&', StringSplitOptions.RemoveEmptyEntries)
                .Select(p => p.Split('=', 2)).ToDictionary(p => p[0], p => p.Length > 1 ? p[1] : "");
            if (path == "/api/sync/pull")
            {
                BeforePull?.Invoke();
                lock (_gate)
                {
                    Pulls++;
                    if (FailNextPull is int status)
                    {
                        FailNextPull = null;
                        return Task.FromResult(Json(status, "{\"error\":{\"code\":\"RESYNC_REQUIRED\",\"message\":\"x\"}}"));
                    }
                    var since = ulong.Parse(query["since"]);
                    if (since > 0 && since < Retained)
                        return Task.FromResult(Json(410, "{\"error\":{\"code\":\"RESYNC_REQUIRED\",\"message\":\"x\"}}"));
                    var changes = _log.Where(c => c["seq"].AsULong() > since && Visible(c["row_id"].AsString(), token))
                        .Take(PageSize).ToList();
                    var last = changes.Count > 0 ? changes[^1]["seq"].AsULong() : Seq;
                    var more = _log.Any(c => c["seq"].AsULong() > last);
                    var body = PylonValue.Object(("changes", PylonValue.Array(changes)),
                        ("cursor", PylonValue.Object(("last_seq", more ? last : Seq))), ("has_more", more));
                    return Task.FromResult(Json(200, body.ToJson()));
                }
            }
            if (path.StartsWith("/api/entities/") && path.EndsWith("/cursor"))
            {
                BeforeCursor?.Invoke();
                var entity = path.Split('/')[3];
                lock (_gate)
                {
                    var rows = _rows.TryGetValue(entity, out var t)
                        ? t.Values.Where(r => Visible(r["id"].AsString(), token)).ToList()
                        : new List<PylonValue>();
                    var body = PylonValue.Object(("data", PylonValue.Array(rows)), ("next_cursor", PylonValue.Null), ("has_more", false));
                    return Task.FromResult(Json(200, body.ToJson()));
                }
            }
            return Task.FromResult(Json(404, "{}"));
        }

        static PylonHttpResponse Json(int status, string body) => new PylonHttpResponse(status, Encoding.UTF8.GetBytes(body));
    }

    public class LiveQueryTests
    {
        static (PylonClient Client, LiveSync Sync, FakeSyncServer Server) Make()
        {
            var server = new FakeSyncServer();
            var client = new PylonClient(new PylonClientOptions(new Uri("http://localhost:4321"))
            {
                Transport = server,
                Dispatcher = PylonDispatcher.Inline,
            });
            client.SetSession("tok");
            return (client, new LiveSync(client, client.Options.Live), server);
        }

        static PylonValue Data(string title) => PylonValue.Object(("title", title));

        /// <summary>packages/sync/conformance: the scenarios that cover reads (no optimistic writes).</summary>
        public static IEnumerable<object[]> Scenarios()
        {
            var dir = Path.Combine(AppContext.BaseDirectory, "fixtures", "conformance");
            foreach (var file in Directory.GetFiles(dir, "*.json").OrderBy(f => f))
            {
                var steps = PylonValue.Parse(File.ReadAllText(file))["steps"].Items;
                if (steps.Any(s => s["op"].AsString() is "update" or "delete")) continue;
                yield return new object[] { Path.GetFileName(file) };
            }
        }

        [Theory]
        [MemberData(nameof(Scenarios))]
        public async Task SyncConformance(string file)
        {
            var (client, sync, server) = Make();
            var scenario = PylonValue.Parse(File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "fixtures", "conformance", file)));
            var i = 0;
            foreach (var step in scenario["steps"].Items)
            {
                i++;
                var where = $"{file} step {i} ({step["op"].AsString()})";
                switch (step["op"].AsString())
                {
                    case "seed":
                        server.Seed(step["entity"].AsString(), step["row_id"].AsString(), step["kind"].AsString(),
                            step["data"].IsNull ? null : step["data"]);
                        break;
                    case "pull":
                        await sync.PullAsync();
                        break;
                    case "frame":
                        sync.OnText(step["frame"].ToJson());
                        break;
                    case "expectRow":
                        var row = sync.Row(step["entity"].AsString(), step["id"].AsString());
                        Assert.True(step["present"].AsBool() == (row != null), where);
                        foreach (var f in step["fields"].Fields) Assert.True(f.Value.Equals(row![f.Key]), $"{where}: {f.Key} = {row[f.Key]}");
                        break;
                    case "expectCount":
                        Assert.True(step["count"].AsInt() == sync.Count(step["entity"].AsString()), where);
                        break;
                    case "expectCursor":
                        Assert.True(step["last_seq"].AsULong() == sync.Cursor, $"{where}: cursor {sync.Cursor}");
                        break;
                    default:
                        throw new InvalidOperationException($"{where}: unknown op");
                }
            }
            client.Dispose();
        }

        [Fact]
        public void TheReadScenariosAreAllRun()
        {
            // Four of the six shared scenarios are reads; the other two test optimistic writes.
            Assert.Equal(4, Scenarios().Count());
        }

        [Fact]
        public async Task APullPagesThroughTheWholeLog()
        {
            var (_, sync, server) = Make();
            server.PageSize = 3;
            for (var n = 0; n < 10; n++) server.Seed("Note", $"n{n}", "insert", Data($"t{n}"));
            await sync.PullAsync();
            Assert.Equal(10, sync.Count("Note"));
            Assert.Equal(10UL, sync.Cursor);
            Assert.True(server.Pulls >= 4);
        }

        [Fact]
        public async Task AFrameDuringAPullWaitsAndAppliesAfterIt()
        {
            var (_, sync, server) = Make();
            server.Seed("Note", "n1", "insert", Data("one"));
            server.Seed("Note", "n2", "insert", Data("two"));
            // While the pull is in flight, seq 3 arrives live; the pull's page stops at 2.
            server.BeforePull = () =>
            {
                server.BeforePull = null;
                sync.OnText(PylonValue.Object(("seq", 3), ("entity", "Note"), ("row_id", "n3"), ("kind", "insert"),
                    ("data", Data("three")), ("timestamp", "")).ToJson());
            };
            await sync.PullAsync();
            Assert.Equal(3, sync.Count("Note"));
            Assert.Equal(3UL, sync.Cursor);
        }

        [Fact]
        public async Task A410StartsTheReplicaOverFromASnapshot()
        {
            var (_, sync, server) = Make();
            server.Seed("Note", "n1", "insert", Data("one"));
            await sync.PullAsync();
            server.Seed("Note", "n2", "insert", Data("two"));
            server.Seed("Note", "n1", "delete");
            server.FailNextPull = 410;
            await sync.PullAsync();
            Assert.False(sync.HasRow("Note", "n1"));
            Assert.True(sync.HasRow("Note", "n2"));
            Assert.Equal(3UL, sync.Cursor);
        }

        [Fact]
        public async Task AnotherTokenStartsTheReplicaOver()
        {
            var (client, sync, server) = Make();
            server.Seed("Note", "n1", "insert", Data("one"));
            await sync.PullAsync();
            server.Hidden.Add("n1");
            client.SetSession("someone-else");
            await sync.PullAsync();
            Assert.False(sync.HasRow("Note", "n1"));
        }

        [Fact]
        public async Task QueriesFilterSortLimitAndReportChanges()
        {
            var (client, _, server) = Make();
            server.Seed("Player", "a", "insert", PylonValue.Object(("party", "p1"), ("score", 5)));
            server.Seed("Player", "b", "insert", PylonValue.Object(("party", "p1"), ("score", 9)));
            server.Seed("Player", "c", "insert", PylonValue.Object(("party", "p2"), ("score", 7)));
            var sync = new LiveSync(client, client.Options.Live);
            var q = new LiveQuery(sync, "Player", new LiveQueryOptions
            {
                Where = PylonValue.Object(("party", "p1")),
                OrderBy = "score",
                Descending = true,
            });
            var changes = new List<LiveQueryChange>();
            q.Changed += changes.Add;
            await sync.PullAsync();
            q.Deliver(q.Select(new[] { sync.Row("Player", "a")!, sync.Row("Player", "b")!, sync.Row("Player", "c")! }), true);
            Assert.Equal(new[] { "b", "a" }, q.Rows.Select(r => r["id"].AsString()));
            Assert.Equal(new[] { "b", "a" }, changes[0].Added.OrderBy(x => x == "a").ToArray());

            // a moves to p2 and b's score drops: a leaves, b updates and stays.
            q.Deliver(q.Select(new[]
            {
                PylonValue.Object(("id", "a"), ("party", "p2"), ("score", 5)),
                PylonValue.Object(("id", "b"), ("party", "p1"), ("score", 1)),
            }), true);
            Assert.Equal(new[] { "a" }, changes[1].Removed);
            Assert.Equal(new[] { "b" }, changes[1].Updated);
            Assert.Equal("b", q.Get("b")!["id"].AsString());

            var top = new LiveQuery(sync, "Player", new LiveQueryOptions { OrderBy = "score", Limit = 1, Filter = r => r["score"].AsInt() > 4 });
            Assert.Equal(new[] { "a" }, top.Select(new[] { sync.Row("Player", "a")!, sync.Row("Player", "c")! }).Select(r => r["id"].AsString()));
            client.Dispose();
        }

        static PylonValue Change(ulong seq, string id, string kind, PylonValue? data = null) =>
            PylonValue.Object(("seq", seq), ("entity", "Note"), ("row_id", id), ("kind", kind),
                ("data", data ?? PylonValue.Null), ("timestamp", ""));

        /// <summary>A client whose HTTP goes to the fake sync server and whose live socket goes to a local WebSocket server.</summary>
        static (PylonClient Client, FakeSyncServer Sync, FakeShardServer Ws) Wired()
        {
            var sync = new FakeSyncServer();
            var ws = new FakeShardServer { Subprotocol = "*" };
            var client = new PylonClient(new PylonClientOptions(new Uri("http://localhost:4321"))
            {
                Transport = sync,
                Dispatcher = PylonDispatcher.Inline,
            });
            client.Options.Live.WsUrl = ws.Url("x");
            client.Options.Live.ReconnectBaseDelay = TimeSpan.FromMilliseconds(20);
            client.SetSession("tok-a");
            return (client, sync, ws);
        }

        static void Until(Func<bool> done, string what)
        {
            var deadline = DateTime.UtcNow.AddSeconds(10);
            while (!done())
            {
                Assert.True(DateTime.UtcNow < deadline, $"timed out waiting for {what}");
                Thread.Sleep(5);
            }
        }

        [Fact]
        public void AFrameTheSocketSendsBeforeTheFirstPullDoesNotSkipTheSnapshot()
        {
            var (client, sync, ws) = Wired();
            using var _ = ws;
            sync.Seed("Note", "n1", "insert", Data("one"));
            sync.Seed("Note", "n2", "insert", Data("two"));
            sync.Seed("Note", "n3", "insert", Data("three"));
            // The socket's first message is seq 3, before any pull has run.
            ws.OnConnection = async (ctx, socket) =>
            {
                await socket.SendAsync(Encoding.UTF8.GetBytes(Change(3, "n3", "insert", Data("three")).ToJson()),
                    System.Net.WebSockets.WebSocketMessageType.Text, true, default);
                await FakeShardServer.Receive(socket);
            };
            using var q = client.Live("Note");
            Until(() => q.Synced, "the first pull");
            Assert.Equal(new[] { "n1", "n2", "n3" }, q.Rows.Select(r => r["id"].AsString()));
            client.Dispose();
        }

        [Fact]
        public void AFailedPullIsRetriedWhileTheSocketStaysOpen()
        {
            var (client, sync, ws) = Wired();
            using var _ = ws;
            sync.Seed("Note", "n1", "insert", Data("one"));
            sync.FailNextPull = 503;
            var connections = 0;
            ws.OnConnection = async (ctx, socket) =>
            {
                Interlocked.Increment(ref connections);
                await FakeShardServer.Receive(socket);
            };
            using var q = client.Live("Note");
            Until(() => q.Get("n1") != null, "the retried pull");
            Assert.Equal(1, Volatile.Read(ref connections));
            client.Dispose();
        }

        [Fact]
        public void ANewSessionStartsOverAndReconnectsWithTheNewToken()
        {
            var (client, sync, ws) = Wired();
            using var _ = ws;
            sync.Seed("Note", "a1", "insert", Data("alice's"));
            sync.OwnerToken["a1"] = "tok-a";
            ws.OnConnection = async (ctx, socket) => await FakeShardServer.Receive(socket);
            using var q = client.Live("Note");
            Until(() => q.Get("a1") != null, "alice's row");
            client.SetSession("tok-b");
            Until(() => q.Rows.Count == 0, "the reset");
            Until(() => ws.Requests.Count >= 2, "the reconnect");
            Assert.Contains("bearer.tok-b", ws.Requests[^1].Headers["Sec-WebSocket-Protocol"]);
            Until(() => q.Synced, "the pull under the new token");
            Assert.Null(q.Get("a1"));
            client.Dispose();
        }

        [Fact]
        public void ASessionChangeDuringTheConnectDropsTheOldSocket()
        {
            var (client, sync, ws) = Wired();
            using var _ = ws;
            sync.Seed("Note", "a1", "insert", Data("alice's"));
            sync.OwnerToken["a1"] = "tok-a";
            ws.AcceptDelay = TimeSpan.FromMilliseconds(300);
            var oldSocketSent = 0;
            ws.OnConnection = async (ctx, socket) =>
            {
                // On a socket opened with alice's token, push alice's change.
                if (ctx.Request.Headers["Sec-WebSocket-Protocol"]!.Contains("tok-a"))
                {
                    Interlocked.Increment(ref oldSocketSent);
                    await socket.SendAsync(Encoding.UTF8.GetBytes(Change(9, "a2", "insert", Data("alice again")).ToJson()),
                        System.Net.WebSockets.WebSocketMessageType.Text, true, default);
                }
                await FakeShardServer.Receive(socket);
            };
            using var q = client.Live("Note");
            Thread.Sleep(100); // the first connect is waiting for the accept
            client.SetSession("tok-b");
            Until(() => ws.Requests.Count >= 2, "the reconnect with the new token");
            Until(() => q.Synced, "the pull under the new token");
            Thread.Sleep(300);
            Assert.Contains("bearer.tok-b", ws.Requests[^1].Headers["Sec-WebSocket-Protocol"]);
            Assert.Null(q.Get("a1"));
            Assert.Null(q.Get("a2"));
            Assert.False(client.LiveEngine!.HasRow("Note", "a2"));
            client.Dispose();
        }

        [Fact]
        public void ASessionChangedFrameRetriesItsPull()
        {
            var (client, sync, ws) = Wired();
            using var _ = ws;
            sync.Seed("Note", "n1", "insert", Data("one"));
            var connections = 0;
            System.Net.WebSockets.WebSocket? server = null;
            ws.OnConnection = async (ctx, socket) =>
            {
                Interlocked.Increment(ref connections);
                server = socket;
                await FakeShardServer.Receive(socket);
            };
            using var q = client.Live("Note");
            Until(() => q.Get("n1") != null, "the first pull");
            sync.FailNextPull = 503;
            server!.SendAsync(Encoding.UTF8.GetBytes("{\"type\":\"session-changed\"}"),
                System.Net.WebSockets.WebSocketMessageType.Text, true, default).Wait();
            sync.Seed("Note", "n2", "insert", Data("two"));
            Until(() => q.Get("n1") != null && q.Get("n2") != null, "the retried pull");
            Assert.Equal(1, Volatile.Read(ref connections));
            client.Dispose();
        }

        [Fact]
        public void AnOlderResultNeverReplacesANewerOne()
        {
            var (client, sync, _) = Make();
            var q = sync.Attach("Note", new LiveQueryOptions());
            q.Deliver(new[] { PylonValue.Object(("id", "n1"), ("title", "new")) }, true, 2);
            q.Deliver(new[] { PylonValue.Object(("id", "n1"), ("title", "old")) }, true, 1);
            Assert.Equal("new", q.Get("n1")!["title"].AsString());
            client.Dispose();
        }

        [Fact]
        public async Task ASweepThatAChangeInvalidatesRunsAgain()
        {
            var (client, sync, server) = Make();
            server.Seed("Note", "n1", "insert", Data("one"));
            server.Seed("Note", "n2", "insert", Data("two"));
            var q = sync.Attach("Note", new LiveQueryOptions());
            await sync.PullAsync();
            // n1 was revoked while the socket was down; during the first sweep a change to n2 lands.
            server.Hidden.Add("n1");
            server.BeforeCursor = () =>
            {
                server.BeforeCursor = null;
                sync.OnText(Change(3, "n2", "update", Data("two-v2")).ToJson());
            };
            await sync.ReconcileForTestAsync();
            Assert.False(sync.HasRow("Note", "n1"));
            Assert.Equal("two-v2", sync.Row("Note", "n2")!["title"].AsString());
            client.Dispose();
        }

        [Fact]
        public async Task ALiveQueryGetsTheInitialRowsAndLaterChangesOnTheDispatcher()
        {
            var (client, sync, server) = Make();
            server.Seed("Note", "n1", "insert", Data("one"));
            // The pull and frames drive the engine; its socket loop does not run.
            var q = sync.Attach("Note", new LiveQueryOptions());
            var changes = new List<LiveQueryChange>();
            q.Changed += changes.Add;
            await sync.PullAsync();
            Assert.True(q.Synced);
            Assert.Equal(new[] { "n1" }, q.Rows.Select(r => r["id"].AsString()));
            sync.OnText(PylonValue.Object(("seq", 2), ("entity", "Note"), ("row_id", "n1"), ("kind", "update"),
                ("data", Data("one-v2")), ("timestamp", "")).ToJson());
            Assert.Equal("one-v2", q.Get("n1")!["title"].AsString());
            Assert.Equal(new[] { "n1" }, changes[^1].Updated);
            sync.OnText("{\"type\":\"row-revoked\",\"entity\":\"Note\",\"row_id\":\"n1\",\"seq\":3}");
            Assert.Empty(q.Rows);
            Assert.Equal(new[] { "n1" }, changes[^1].Removed);
            client.Dispose();
        }
    }
}
