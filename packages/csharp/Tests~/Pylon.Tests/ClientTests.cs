using System;
using System.IO;
using System.Linq;
using System.Threading.Tasks;
using Pylon;
using Xunit;

namespace Pylon.Tests
{
    public class ClientTests
    {
        static (PylonClient Client, MockTransport Http) Make(IPylonStorage? storage = null)
        {
            var http = new MockTransport();
            var client = new PylonClient(new PylonClientOptions(new Uri("http://localhost:4321"))
            {
                Transport = http,
                Storage = storage ?? new MemoryStorage(),
                Dispatcher = PylonDispatcher.Inline,
            });
            return (client, http);
        }

        [Fact]
        public async Task GuestSignInStoresTheTokenAndLaterRequestsSendIt()
        {
            var (client, http) = Make();
            http.Reply(201, "{\"token\":\"tok_1\",\"user_id\":\"guest_1\",\"guest\":true}")
                .Reply(200, "{\"x\":1}");
            var session = await client.SignInAsGuestAsync();
            Assert.Equal("tok_1", session.Token);
            Assert.Equal("guest_1", session.UserId);
            Assert.True(session.Guest);
            Assert.Equal("tok_1", client.Token);
            Assert.Equal("POST", http.Requests[0].Method);
            Assert.Equal("/api/auth/guest", http.Requests[0].Url.AbsolutePath);
            Assert.False(http.Requests[0].Headers.ContainsKey("Authorization"));

            var result = await client.CallFnAsync("joinArena", PylonValue.Object(("arena", "a1")));
            Assert.Equal(1, result["x"].AsInt());
            var call = http.Requests[1];
            Assert.Equal("/api/fn/joinArena", call.Url.AbsolutePath);
            Assert.Equal("Bearer tok_1", call.Headers["Authorization"]);
            Assert.Equal("application/json", call.Headers["Content-Type"]);
            Assert.Equal("{\"arena\":\"a1\"}", MockTransport.BodyOf(call));
        }

        [Fact]
        public async Task SignInRoutesMatchTheServer()
        {
            var (client, http) = Make();
            const string ok = "{\"token\":\"t\",\"user_id\":\"u\",\"expires_at\":1900000000}";
            http.Reply(200, "{\"sent\":true}").Reply(200, ok).Reply(200, ok).Reply(200, ok).Reply(200, ok);
            await client.StartMagicCodeAsync("a@b.co");
            var magic = await client.VerifyMagicCodeAsync("a@b.co", "123456");
            await client.SignInWithPasswordAsync("a@b.co", "pw");
            await client.RegisterWithPasswordAsync("a@b.co", "pw");
            var refreshed = await client.RefreshSessionAsync();
            Assert.Equal(1900000000L, magic.ExpiresAt);
            Assert.Equal(1900000000L, refreshed.ExpiresAt);
            Assert.Equal(
                new[] { "/api/auth/magic/send", "/api/auth/magic/verify", "/api/auth/password/login", "/api/auth/password/register", "/api/auth/refresh" },
                http.Requests.Select(r => r.Url.AbsolutePath).ToArray());
            Assert.Equal("{\"email\":\"a@b.co\",\"code\":\"123456\"}", MockTransport.BodyOf(http.Requests[1]));
            Assert.Null(http.Requests[4].Body);
        }

        [Fact]
        public async Task ErrorsCarryTheServerCodeAndMessage()
        {
            var (client, http) = Make();
            http.Reply(401, "{\"error\":{\"code\":\"UNAUTHENTICATED\",\"message\":\"sign in first\"}}")
                .Reply(429, "{\"code\":\"RATE_LIMITED\",\"message\":\"slow down\"}")
                .Reply(502, "Bad Gateway")
                .Fail("connection refused");
            var e1 = await Assert.ThrowsAsync<PylonException>(() => client.CallFnAsync("f"));
            Assert.Equal(PylonErrorKind.Http, e1.Kind);
            Assert.Equal(401, e1.Status);
            Assert.Equal("UNAUTHENTICATED", e1.Code);
            Assert.Equal("sign in first", e1.ServerMessage);
            var e2 = await Assert.ThrowsAsync<PylonException>(() => client.CallFnAsync("f"));
            Assert.Equal("RATE_LIMITED", e2.Code);
            var e3 = await Assert.ThrowsAsync<PylonException>(() => client.CallFnAsync("f"));
            Assert.Null(e3.Code);
            Assert.Equal("Bad Gateway", e3.ServerMessage);
            var e4 = await Assert.ThrowsAsync<PylonException>(() => client.CallFnAsync("f"));
            Assert.Equal(PylonErrorKind.Transport, e4.Kind);
        }

        [Fact]
        public async Task EntityRoutesAndEnvelopes()
        {
            var (client, http) = Make();
            http.Reply(200, "{\"count\":2,\"data\":[{\"id\":\"a\"},{\"id\":\"b\"}],\"limit\":100,\"offset\":0}")
                .Reply(200, "[{\"id\":\"c\"}]")
                .Reply(200, "{\"data\":[{\"id\":\"d\"}],\"next_cursor\":\"cur 2\",\"has_more\":true}")
                .Reply(200, "{\"id\":\"a b\"}")
                .Reply(201, "{\"id\":\"n\"}")
                .Reply(200, "{\"id\":\"n\",\"hp\":3}")
                .Reply(200, "");
            var rows = await client.ListAsync("Character");
            Assert.Equal(new[] { "a", "b" }, rows.Select(r => r["id"].AsString()));
            Assert.Single(await client.ListAsync("Character"));
            var page = await client.ListCursorAsync("Character", after: "cur 1", limit: 10);
            Assert.True(page.HasMore);
            Assert.Equal("cur 2", page.NextCursor);
            await client.GetAsync("Character", "a b");
            await client.CreateAsync("Character", PylonValue.Object(("hp", 1)));
            await client.UpdateAsync("Character", "n", PylonValue.Object(("hp", 3)));
            await client.DeleteAsync("Character", "n");
            Assert.Equal("/api/entities/Character/cursor?limit=10&after=cur%201", http.Requests[2].Url.PathAndQuery);
            Assert.Equal("/api/entities/Character/a%20b", http.Requests[3].Url.AbsolutePath);
            Assert.Equal(new[] { "GET", "GET", "GET", "GET", "POST", "PATCH", "DELETE" }, http.Requests.Select(r => r.Method));
        }

        [Fact]
        public async Task TypedCallsUseTheConverters()
        {
            var (client, http) = Make();
            http.Reply(200, "{\"sum\":5}");
            var args = PylonConverter.Create<(int A, int B)>(p => PylonValue.Object(("a", p.A), ("b", p.B)), v => default);
            var result = PylonConverter.Create<int>(i => i, v => v["sum"].AsInt());
            Assert.Equal(5, await client.CallFnAsync("add", (2, 3), args, result));
            Assert.Equal("{\"a\":2,\"b\":3}", MockTransport.BodyOf(http.Requests[0]));

            http.Reply(200, "{\"sum\":\"five\"}");
            var e = await Assert.ThrowsAsync<PylonException>(() => client.CallFnAsync("add", (2, 3), args, result));
            Assert.Equal(PylonErrorKind.Decoding, e.Kind);
        }

        [Fact]
        public async Task FunctionNamesKeepTheirSlashes()
        {
            var (client, http) = Make();
            http.Reply(200, "null");
            Assert.True((await client.CallFnAsync("billing/charge card")).IsNull);
            Assert.Equal("/api/fn/billing/charge%20card", http.Requests[0].Url.AbsolutePath);
        }

        [Fact]
        public async Task LogoutClearsTheTokenEvenWhenTheServerFails()
        {
            var (client, http) = Make();
            client.SetSession("t");
            http.Fail("down");
            await Assert.ThrowsAsync<PylonException>(() => client.LogoutAsync());
            Assert.Null(client.Token);
        }

        [Fact]
        public void TokenKeysMatchTheOtherClients()
        {
            Assert.Equal("pylon_token", StorageKeys.Token());
            Assert.Equal("pylon:trux:token", StorageKeys.Token("trux"));
        }

        [Fact]
        public void FileStorageKeepsValuesBetweenInstances()
        {
            var path = Path.Combine(Path.GetTempPath(), $"pylon-storage-{Guid.NewGuid():N}", "s.json");
            var a = new FileStorage(path);
            a.Set("pylon_token", "abc");
            a.Set("other", "x");
            a.Remove("other");
            var b = new FileStorage(path);
            Assert.Equal("abc", b.Get("pylon_token"));
            Assert.Null(b.Get("other"));
            File.WriteAllText(path, "not json");
            Assert.Null(new FileStorage(path).Get("pylon_token"));
        }

        [Fact]
        public async Task AutoRefreshClearsTheTokenWhenTheServerRefusesIt()
        {
            var (client, http) = Make();
            client.SetSession("old");
            http.Reply(401, "{\"error\":{\"code\":\"SESSION_EXPIRED\",\"message\":\"expired\"}}");
            var expired = new TaskCompletionSource<bool>();
            using var handle = client.StartSessionAutoRefresh(
                DateTimeOffset.UtcNow.ToUnixTimeSeconds(), onExpired: () => expired.TrySetResult(true));
            Assert.True(await Task.WhenAny(expired.Task, Task.Delay(5000)) == expired.Task);
            Assert.Null(client.Token);
        }

        [Fact]
        public async Task AutoRefreshStoresEachNewToken()
        {
            var (client, http) = Make();
            client.SetSession("old");
            var far = DateTimeOffset.UtcNow.ToUnixTimeSeconds() + 30 * 86400;
            http.Reply(200, $"{{\"token\":\"new\",\"user_id\":\"u\",\"expires_at\":{far}}}");
            var refreshed = new TaskCompletionSource<SessionResponse>();
            using var handle = client.StartSessionAutoRefresh(
                DateTimeOffset.UtcNow.ToUnixTimeSeconds() + 10, onRefresh: s => refreshed.TrySetResult(s),
                margin: TimeSpan.FromSeconds(60));
            Assert.True(await Task.WhenAny(refreshed.Task, Task.Delay(5000)) == refreshed.Task);
            Assert.Equal("new", client.Token);
            Assert.Equal("Bearer old", http.Requests[0].Headers["Authorization"]);
        }
    }
}
