using System;
using System.Collections.Concurrent;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using Pylon;
using Xunit;

namespace Pylon.Tests
{
    /// <summary>
    /// Live queries against a running server: <c>pylon dev</c> in
    /// examples/todo-app (its Todo entity is readable only by its owner,
    /// <c>auth.userId == data.userId</c>), with <c>PYLON_SYNC_TEST_URL</c>
    /// set to its origin.
    /// </summary>
    public class LiveQueryLiveTests
    {
        const string Env = "PYLON_SYNC_TEST_URL";

        static async Task<(PylonClient Client, string UserId)> SignedIn()
        {
            var client = new PylonClient(new PylonClientOptions(new Uri(Environment.GetEnvironmentVariable(Env)!))
            {
                Dispatcher = PylonDispatcher.Inline,
            });
            client.Options.Live.ReconnectBaseDelay = TimeSpan.FromMilliseconds(50);
            var session = await client.SignInAsGuestAsync();
            return (client, session.UserId!);
        }

        static PylonValue Todo(string userId, string title) => PylonValue.Object(
            ("userId", userId), ("title", title), ("done", false), ("priority", "med"),
            ("createdAt", DateTime.UtcNow.ToString("o")));

        static void Until(Func<bool> done, string what, int seconds = 15)
        {
            var deadline = DateTime.UtcNow.AddSeconds(seconds);
            while (!done())
            {
                Assert.True(DateTime.UtcNow < deadline, $"timed out waiting for {what}");
                Thread.Sleep(10);
            }
        }

        [LiveFact(Env)]
        public async Task ACreateUpdateAndDeleteReachRowsAndChanged()
        {
            var (client, me) = await SignedIn();
            using var _ = client;
            using var q = client.Live("Todo", new LiveQueryOptions { Where = PylonValue.Object(("userId", me)) });
            var changes = new ConcurrentQueue<LiveQueryChange>();
            q.Changed += changes.Enqueue;
            Until(() => q.Synced, "the first pull");
            Assert.Empty(q.Rows);

            var created = await client.CreateAsync("Todo", Todo(me, "buy potions"));
            var id = created["id"].AsString();
            Until(() => q.Get(id) != null, "the insert");
            Assert.Contains(changes, c => c.Added.Contains(id));

            await client.UpdateAsync("Todo", id, PylonValue.Object(("title", "buy elixirs")));
            Until(() => q.Get(id)?["title"].AsStringOr(null) == "buy elixirs", "the update");
            Assert.Contains(changes, c => c.Updated.Contains(id));

            await client.DeleteAsync("Todo", id);
            Until(() => q.Get(id) == null, "the delete");
            Assert.Contains(changes, c => c.Removed.Contains(id));
        }

        [LiveFact(Env)]
        public async Task AReconnectInTheMiddleOfChangesLosesNone()
        {
            var (client, me) = await SignedIn();
            using var _ = client;
            using var q = client.Live("Todo", new LiveQueryOptions { Where = PylonValue.Object(("userId", me)) });
            Until(() => q.Synced && client.LiveEngine!.Connected, "the socket");

            var ids = new ConcurrentBag<string>();
            var writer = Task.Run(async () =>
            {
                for (var i = 0; i < 20; i++)
                {
                    ids.Add((await client.CreateAsync("Todo", Todo(me, $"t{i}")))["id"].AsString());
                    await Task.Delay(15);
                }
            });
            // Drop the socket twice while the writes land.
            await Task.Delay(60);
            client.LiveEngine!.DropSocketForTest();
            await Task.Delay(120);
            client.LiveEngine!.DropSocketForTest();
            await writer;
            Until(() => ids.All(id => q.Get(id) != null), "every write after the reconnects");
            Assert.Equal(20, q.Rows.Count);
        }

        [LiveFact(Env)]
        public async Task ARowThePolicyHidesNeverArrives()
        {
            var (alice, aliceId) = await SignedIn();
            var (bob, bobId) = await SignedIn();
            using var a = alice;
            using var b = bob;
            using var bobsView = bob.Live("Todo");
            var seen = new ConcurrentBag<string>();
            bobsView.Changed += _ =>
            {
                foreach (var r in bobsView.Rows) seen.Add(r["userId"].AsString());
            };
            Until(() => bobsView.Synced && bob.LiveEngine!.Connected, "bob's socket");

            var secret = await alice.CreateAsync("Todo", Todo(aliceId, "alice only"));
            var mine = await bob.CreateAsync("Todo", Todo(bobId, "bob's"));
            // Bob's own write arrives, so the socket works; Alice's never does.
            Until(() => bobsView.Get(mine["id"].AsString()) != null, "bob's own row");
            await Task.Delay(300);
            Assert.Null(bobsView.Get(secret["id"].AsString()));
            Assert.DoesNotContain(aliceId, seen);
            Assert.False(bob.LiveEngine!.HasRow("Todo", secret["id"].AsString()));
        }
    }
}
