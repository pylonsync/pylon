#nullable enable
using System;
using System.Threading;
using System.Threading.Tasks;
using Pylon;
using Pylon.Realtime;
using UnityEngine;

namespace Pylon.Samples.Arena
{
    /// <summary>Extra checks for the Arena Web build. Uses the same SDK in native builds.</summary>
    public sealed class WebValidation : MonoBehaviour
    {
        [Tooltip("Optional todo-app origin for live-query validation.")]
        public string liveQueryUrl = "http://localhost:4435";
        readonly CancellationTokenSource _stop = new CancellationTokenSource();
        PylonClient? _client;
        PylonClient? _todos;
        ShardGame<PylonValue>? _frontier;
        LiveQuery? _query;
        string _status = "starting";
        bool _connecting;
        int _frames;
        int _cycle;
        float _nextLog;
        float _nextMove;

        async void Start()
        {
            try
            {
                _client = new PylonClient(GetComponent<ArenaSample>().serverUrl);
                await _client.SignInAsGuestAsync(_stop.Token);
                await ConnectFrontier();
                if (!string.IsNullOrEmpty(liveQueryUrl)) await CheckLiveQuery();
            }
            catch (OperationCanceledException) { }
            catch (Exception e) { _status = e.Message; Debug.LogError("PYLON_WEB " + e); }
        }

        async Task ConnectFrontier()
        {
            _frontier?.Dispose();
            _frontier = null;
            var join = await _client!.CallFnAsync("joinFrontier", PylonValue.Object(("frontier", "frontier-web"), ("size", 500)), _stop.Token);
            _stop.Token.ThrowIfCancellationRequested();
            _frontier = new ShardGame<PylonValue>(join["shardId"].AsString(), new ShardConnectionOptions
            {
                BaseUrl = _client.BaseUrl,
                SubscriberId = join["subscriberId"].AsString(),
                TicketProvider = async (_, ct) => (await _client.CallFnAsync("joinFrontier", PylonValue.Object(("frontier", "frontier-web"), ("size", 500)), ct))["ticket"].AsString(),
                IdleTimeout = TimeSpan.FromSeconds(5), TickRate = 20,
            }, PylonConverter.Value);
            _frontier.Opened += () => _frontier.Send("join");
            _frontier.Replication += _ => _frames++;
            _frontier.Error += e => Debug.LogWarning("PYLON_WEB frontier " + e.Message);
            _frontier.Connect();
            _cycle++;
        }

        async Task CheckLiveQuery()
        {
            _todos = new PylonClient(liveQueryUrl);
            var session = await _todos.SignInAsGuestAsync(_stop.Token);
            _query = _todos.Live("Todo", new LiveQueryOptions { Where = PylonValue.Object(("userId", session.UserId)) });
            var row = await _todos.CreateAsync("Todo", PylonValue.Object(("userId", session.UserId), ("title", "Unity Web validation"), ("done", false), ("priority", "med"), ("createdAt", DateTime.UtcNow.ToString("o"))), _stop.Token);
            var id = row["id"].AsString();
            await Until(() => _query.Get(id) != null);
            await _todos.UpdateAsync("Todo", id, PylonValue.Object(("title", "updated")), _stop.Token);
            await Until(() => _query.Get(id)?["title"].AsStringOr(null) == "updated");
            await _todos.DeleteAsync("Todo", id, _stop.Token);
            await Until(() => _query.Get(id) == null);
            _status = "live query create/update/delete passed";
            Debug.Log("PYLON_WEB " + _status);
        }

        async Task Until(Func<bool> done)
        {
            var start = Time.realtimeSinceStartup;
            while (!done())
            {
                _stop.Token.ThrowIfCancellationRequested();
                if (Time.realtimeSinceStartup - start > 20) throw new TimeoutException("Live query update timed out");
                await Task.Yield();
            }
        }

        void Update()
        {
            _frontier?.Frame();
            if (_frontier?.Connected == true && Time.time >= _nextMove)
            {
                _frontier.Send("join");
                _frontier.Send(PylonValue.Object(("move_to", PylonValue.Object(("x", 200 + 50 * Mathf.Sin(Time.time)), ("y", 200)))));
                _nextMove = Time.time + 1;
            }
            if (Time.time < _nextLog) return;
            _nextLog = Time.time + 5;
            var arena = GetComponent<ArenaSample>();
            Debug.Log($"PYLON_WEB arena={arena.Status} snapshots={arena.SnapshotsReceived} movesAcked={arena.MovesAcked} predicted={arena.PredictedTarget} frontierFrames={_frames} entities={_frontier?.Latest.Count} cycle={_cycle} live={_status}");
        }

        async void Cycle()
        {
            if (_connecting || _client == null) return;
            _connecting = true;
            try { await ConnectFrontier(); }
            catch (OperationCanceledException) { }
            catch (Exception e) { Debug.LogError(e); }
            finally { _connecting = false; }
        }

        void OnGUI()
        {
            GUILayout.BeginArea(new Rect(10, 185, 620, 100), GUI.skin.box);
            GUILayout.Label($"Frontier: {_frames} frames, {_frontier?.Latest.Count} entities, cycle {_cycle}");
            GUILayout.Label(_status);
            if (GUILayout.Button("Disconnect and reconnect frontier")) Cycle();
            GUILayout.EndArea();
        }
        void OnDestroy()
        {
            _stop.Cancel();
            _frontier?.Dispose(); _query?.Dispose(); _client?.Dispose(); _todos?.Dispose();
        }
    }
}
