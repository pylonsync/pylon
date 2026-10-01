using System;
using System.Threading.Tasks;
using Pylon;
using Xunit;

namespace Pylon.Tests
{
    public class NativeSignInTests
    {
        static (PylonClient Client, MockTransport Http) Make()
        {
            var http = new MockTransport();
            var client = new PylonClient(new PylonClientOptions(new Uri("http://localhost:4321"))
            {
                Transport = http,
                Dispatcher = PylonDispatcher.Inline,
            });
            return (client, http);
        }

        const string Session = "{\"token\":\"tok\",\"user_id\":\"u1\",\"expires_at\":1900000000,\"provider\":\"x\"}";

        [Fact]
        public async Task AppleSendsTheIdTokenAndTheFirstSignInName()
        {
            var (client, http) = Make();
            http.Reply(200, Session).Reply(200, Session);
            var s = await client.SignInWithAppleAsync("apple.jwt", "  Jane Doe ");
            await client.SignInWithAppleAsync("apple.jwt");
            Assert.Equal("tok", s.Token);
            Assert.Equal("tok", client.Token);
            Assert.Equal("/api/auth/native/apple", http.Requests[0].Url.AbsolutePath);
            Assert.Equal("POST", http.Requests[0].Method);
            Assert.Equal("{\"id_token\":\"apple.jwt\",\"name\":\"Jane Doe\"}", MockTransport.BodyOf(http.Requests[0]));
            Assert.Equal("{\"id_token\":\"apple.jwt\"}", MockTransport.BodyOf(http.Requests[1]));
        }

        [Fact]
        public async Task GoogleAndSteamUseTheirRoutes()
        {
            var (client, http) = Make();
            http.Reply(200, Session).Reply(200, Session);
            await client.SignInWithGoogleAsync("google.jwt");
            await client.SignInWithSteamAsync(" 14000000abcdef ");
            Assert.Equal("/api/auth/native/google", http.Requests[0].Url.AbsolutePath);
            Assert.Equal("{\"id_token\":\"google.jwt\"}", MockTransport.BodyOf(http.Requests[0]));
            Assert.Equal("/api/auth/native/steam", http.Requests[1].Url.AbsolutePath);
            Assert.Equal("{\"ticket\":\"14000000abcdef\"}", MockTransport.BodyOf(http.Requests[1]));
            Assert.Equal("tok", client.Token);
        }

        [Fact]
        public async Task RefusalsKeepTheOldSessionAndCarryTheServerCode()
        {
            var (client, http) = Make();
            client.SetSession("before");
            http.Reply(401, "{\"error\":{\"code\":\"INVALID_TICKET\",\"message\":\"The Steam ticket was not accepted\"}}")
                .Reply(403, "{\"error\":{\"code\":\"ACCOUNT_BANNED\",\"message\":\"This Steam account is banned\"}}")
                .Reply(502, "{\"error\":{\"code\":\"PROVIDER_UNAVAILABLE\",\"message\":\"Could not reach Steam\"}}");
            var e1 = await Assert.ThrowsAsync<PylonException>(() => client.SignInWithSteamAsync("abcd"));
            Assert.Equal("INVALID_TICKET", e1.Code);
            var e2 = await Assert.ThrowsAsync<PylonException>(() => client.SignInWithSteamAsync("abcd"));
            Assert.Equal(403, e2.Status);
            var e3 = await Assert.ThrowsAsync<PylonException>(() => client.SignInWithSteamAsync("abcd"));
            Assert.Equal("PROVIDER_UNAVAILABLE", e3.Code);
            Assert.Equal("before", client.Token);
        }

        [Fact]
        public async Task EmptyArgumentsAreRefusedBeforeAnyRequest()
        {
            var (client, http) = Make();
            await Assert.ThrowsAsync<PylonException>(() => client.SignInWithAppleAsync(""));
            await Assert.ThrowsAsync<PylonException>(() => client.SignInWithGoogleAsync(" "));
            await Assert.ThrowsAsync<PylonException>(() => client.SignInWithSteamAsync(""));
            Assert.Empty(http.Requests);
        }
    }

    /// <summary>
    /// Against a real server. crates/runtime/tests/native_signin_csharp.rs
    /// starts one with Apple and Google keys it controls and a fake Steam
    /// Web API, and runs these with the URL and tokens in the environment.
    /// </summary>
    public class LiveNativeSignInTests
    {
        const string Env = "PYLON_NATIVE_SIGNIN_URL";

        static PylonClient NewClient() => new PylonClient(new PylonClientOptions(new Uri(Environment.GetEnvironmentVariable(Env)!))
        {
            Dispatcher = PylonDispatcher.Inline,
        });

        static string Var(string name) =>
            Environment.GetEnvironmentVariable(name) ?? throw new InvalidOperationException($"{name} is not set");

        [LiveFact(Env)]
        public async Task AppleSignsInAndTheSessionWorks()
        {
            using var client = NewClient();
            var s = await client.SignInWithAppleAsync(Var("PYLON_TEST_APPLE_ID_TOKEN"), "Jane Doe");
            Assert.Equal(s.Token, client.Token);
            Assert.True(s.ExpiresAt > DateTimeOffset.UtcNow.ToUnixTimeSeconds());
            var me = await client.MeAsync();
            Assert.Equal(s.UserId, me.UserId);
            var row = await client.GetAsync("User", s.UserId!);
            Assert.Equal("Jane Doe", row["displayName"].AsString());
        }

        [LiveFact(Env)]
        public async Task GoogleSignsIn()
        {
            using var client = NewClient();
            var s = await client.SignInWithGoogleAsync(Var("PYLON_TEST_GOOGLE_ID_TOKEN"));
            Assert.Equal(s.UserId, (await client.MeAsync()).UserId);
        }

        [LiveFact(Env)]
        public async Task SteamSignsInTwiceAsTheSameUser()
        {
            using var client = NewClient();
            var first = await client.SignInWithSteamAsync(Var("PYLON_TEST_STEAM_TICKET"));
            Assert.Equal(first.UserId, (await client.MeAsync()).UserId);
            var second = await client.SignInWithSteamAsync(Var("PYLON_TEST_STEAM_TICKET"));
            Assert.Equal(first.UserId, second.UserId);
            Assert.NotEqual(first.Token, second.Token);
        }

        [LiveFact(Env)]
        public async Task SteamRefusesATicketForAnotherIdentity()
        {
            using var client = NewClient();
            var e = await Assert.ThrowsAsync<PylonException>(
                () => client.SignInWithSteamAsync(Var("PYLON_TEST_STEAM_FOREIGN_TICKET")));
            Assert.Equal(401, e.Status);
            Assert.Equal("INVALID_TICKET", e.Code);
            Assert.Null(client.Token);
        }
    }
}
