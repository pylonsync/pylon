#nullable enable
using System;
using System.Collections.Generic;
using System.Threading;
using Pylon;
using Pylon.Realtime;
using Pylon.Unity;
using UnityEngine;

namespace Pylon.Samples.Arena
{
    /// <summary>
    /// Signs in as a guest, calls the <c>joinArena</c> function for a shard
    /// ticket, joins the arena shard through a <see cref="ShardGame{TInput}"/>,
    /// and moves this player: in a circle, or to where you click when the
    /// legacy input manager is on. Every player in the arena is a sphere. A
    /// small cube marks this player's target: the game's predictor moves it
    /// as soon as an input is sent, and each snapshot's ack corrects it.
    ///
    /// Run the server first: <c>pylon dev</c> in examples/shard-arena of
    /// the Pylon repo, then press Play. With <c>PYLON_WEBTRANSPORT_PORT</c>
    /// set on the server, the sample connects over WebTransport.
    /// </summary>
    public sealed class ArenaSample : MonoBehaviour
    {
        [Tooltip("The app's origin: pylon dev prints it.")]
        public string serverUrl = "http://localhost:4321";

        [Tooltip("Which arena to join (a shard per arena).")]
        public string arena = "arena-main";

        [Tooltip("World units per arena unit. The arena is 800 by 500.")]
        public float scale = 0.01f;

        [Tooltip("Auto: WebTransport when the plugin and the app support it, else a WebSocket.")]
        public ShardTransport transport = ShardTransport.Auto;

        /// <summary>What the sample is doing, for the on-screen label.</summary>
        public string Status { get; private set; } = "starting";

        public string? SubscriberId { get; private set; }

        /// <summary>This player's position in arena units, once the shard has it.</summary>
        public Vector2? MyPosition { get; private set; }

        public int SnapshotsReceived { get; private set; }

        /// <summary>Move inputs the shard has acknowledged.</summary>
        public int MovesAcked { get; private set; }

        public string? LastError { get; private set; }

        /// <summary>The transport of the open connection (null before it opens).</summary>
        public ShardTransport? ConnectedOver => _game?.Connection.Transport;

        /// <summary>The target the predictor shows for this player, in arena units.</summary>
        public Vector2? PredictedTarget { get; private set; }

        public Material? sampleMaterial;
        readonly CancellationTokenSource _stop = new CancellationTokenSource();
        PylonClient? _client;
        ShardGame<PylonValue>? _game;
        Predictor<Vector2?, PylonValue>? _target;
        GameObject? _targetMarker;
        readonly Dictionary<string, GameObject> _dots = new Dictionary<string, GameObject>();
        ulong _lastMoveSeq;
        float _nextMove;
        float _nextJoin;
        float _angle;

        async void Start()
        {
            SetUpScene();
            try
            {
                _client = new PylonClient(new PylonClientOptions(new Uri(serverUrl))
                {
                    Storage = new PlayerPrefsStorage(),
                });
                Status = "signing in";
                var session = await _client.SignInAsGuestAsync(_stop.Token);
                SubscriberId = session.UserId;

                Status = "calling joinArena";
                var join = await _client.CallFnAsync("joinArena", PylonValue.Object(("arena", arena)), _stop.Token);

                _stop.Token.ThrowIfCancellationRequested();
                // Created on the main thread, so every event below runs there.
                _game = new ShardGame<PylonValue>(join["shardId"].AsString(), new ShardConnectionOptions
                {
                    BaseUrl = _client.BaseUrl,
                    SubscriberId = join["subscriberId"].AsString(),
                    TicketProvider = async (_, ct) => (await _client.CallFnAsync("joinArena", PylonValue.Object(("arena", arena)), ct))["ticket"].AsString(),
                    TickRate = 20,
                    IdleTimeout = TimeSpan.FromSeconds(5),
                    Transport = transport,
                }, PylonConverter.Value);
                // The predicted target: a move_to sets it, anything else keeps it.
                _target = _game.Predict<Vector2?>((target, input) =>
                    input["move_to"].IsNull
                        ? target
                        : new Vector2(input["move_to"]["x"].AsFloat(), input["move_to"]["y"].AsFloat()));
                _game.Opened += () => { Status = $"connected over {_game.Connection.Transport}"; LastError = null; };
                _game.StateChanged += (state, reason) => Status = reason == null ? state.ToString() : $"{state}: {reason}";
                _game.Connection.Snapshot += OnSnapshot;
                _game.InputRejected += r => LastError = $"input {r.ClientSeq} refused: {r.Code} {r.Message}";
                _game.Error += e => LastError = e.Message;
                Status = "connecting";
                _game.Connect();
            }
            catch (OperationCanceledException) { }
            catch (PylonException e)
            {
                Status = "failed";
                LastError = e.Message;
                Debug.LogError($"[Pylon arena] {e.Message}");
            }
        }

        void OnSnapshot(ShardSnapshot snapshot)
        {
            SnapshotsReceived++;
            if (_lastMoveSeq > 0 && snapshot.Ack >= _lastMoveSeq)
            {
                MovesAcked++;
                _lastMoveSeq = 0;
            }
            var seen = new HashSet<string>();
            foreach (var p in snapshot.State!["players"].Items)
            {
                var id = p["id"].AsString();
                seen.Add(id);
                var pos = new Vector2(p["x"].AsFloat(), p["y"].AsFloat());
                if (!_dots.TryGetValue(id, out var dot))
                {
                    dot = GameObject.CreatePrimitive(PrimitiveType.Sphere);
                    if (sampleMaterial != null) dot.GetComponent<Renderer>().material = sampleMaterial;
                    dot.name = id == SubscriberId ? "You" : id;
                    dot.transform.localScale = Vector3.one * 0.3f;
                    var hue = p["hue"].AsFloat() / 360f;
                    dot.GetComponent<Renderer>().material.color =
                        id == SubscriberId ? Color.green : Color.HSVToRGB(hue, 0.7f, 0.9f);
                    _dots[id] = dot;
                }
                dot.transform.position = ToWorld(pos);
                if (id == SubscriberId)
                {
                    MyPosition = pos;
                    // The server's target as of this snapshot, plus the moves it has not applied yet.
                    PredictedTarget = _target!.Reconcile(new Vector2(p["tx"].AsFloat(), p["ty"].AsFloat()), snapshot.Ack);
                    PlaceTargetMarker();
                }
            }
            foreach (var id in new List<string>(_dots.Keys))
            {
                if (seen.Contains(id)) continue;
                Destroy(_dots[id]);
                _dots.Remove(id);
            }
        }

        void Update()
        {
            if (_game == null || !_game.Connected) return;
            // The arena drops a player who sends nothing for a minute.
            if (Time.time >= _nextJoin)
            {
                _game.Send("join");
                _nextJoin = Time.time + 20;
            }
#if ENABLE_LEGACY_INPUT_MANAGER
            if (Input.GetMouseButtonDown(0) && Camera.main != null)
            {
                var ray = Camera.main.ScreenPointToRay(Input.mousePosition);
                if (new Plane(Vector3.up, Vector3.zero).Raycast(ray, out var d))
                {
                    var hit = ray.GetPoint(d);
                    MoveTo(new Vector2(hit.x / scale, hit.z / scale));
                    _nextMove = Time.time + 5;
                }
            }
#endif
            if (Time.time >= _nextMove)
            {
                _angle += 0.9f;
                MoveTo(new Vector2(400 + 180 * Mathf.Cos(_angle), 250 + 150 * Mathf.Sin(_angle)));
                _nextMove = Time.time + 1.5f;
            }
        }

        void MoveTo(Vector2 target)
        {
            var seq = _game!.Send(PylonValue.Object(("move_to", PylonValue.Object(("x", target.x), ("y", target.y)))));
            // 0: not sent, so the predictor did not record it either.
            if (seq == 0) return;
            _lastMoveSeq = seq;
            PredictedTarget = target;
            PlaceTargetMarker();
        }

        void PlaceTargetMarker()
        {
            if (PredictedTarget == null) return;
            if (_targetMarker == null)
            {
                _targetMarker = GameObject.CreatePrimitive(PrimitiveType.Cube);
                if (sampleMaterial != null) _targetMarker.GetComponent<Renderer>().material = sampleMaterial;
                _targetMarker.name = "Your target";
                _targetMarker.transform.localScale = Vector3.one * 0.12f;
                _targetMarker.GetComponent<Renderer>().material.color = Color.yellow;
            }
            _targetMarker.transform.position = ToWorld(PredictedTarget.Value);
        }

        Vector3 ToWorld(Vector2 p) => new Vector3(p.x * scale, 0.15f, p.y * scale);

        void SetUpScene()
        {
            var ground = GameObject.CreatePrimitive(PrimitiveType.Plane);
            if (sampleMaterial != null) ground.GetComponent<Renderer>().material = sampleMaterial;
            ground.GetComponent<Renderer>().material.color = new Color(0.25f, 0.3f, 0.35f);
            ground.name = "Arena floor";
            // A plane is 10 by 10 units.
            ground.transform.localScale = new Vector3(800 * scale / 10, 1, 500 * scale / 10);
            ground.transform.position = new Vector3(400 * scale, 0, 250 * scale);
            var cam = Camera.main;
            if (cam != null)
            {
                cam.transform.position = new Vector3(400 * scale, 7, -1.5f);
                cam.transform.LookAt(new Vector3(400 * scale, 0, 250 * scale));
            }
        }

        void OnGUI()
        {
            GUILayout.BeginArea(new Rect(10, 10, 520, 160), GUI.skin.box);
            GUILayout.Label($"Pylon arena: {Status}");
            GUILayout.Label($"You: {SubscriberId ?? "-"}  at {(MyPosition.HasValue ? MyPosition.Value.ToString("F0") : "-")}");
            if (_game != null)
                GUILayout.Label($"Tick {_game.Tick}  ack {_game.Ack}  rtt {(_game.RttMs.HasValue ? _game.RttMs.Value.ToString("F0") + " ms" : "-")}  players {_dots.Count}");
            if (LastError != null) GUILayout.Label($"Error: {LastError}");
            GUILayout.EndArea();
        }

        void OnDestroy()
        {
            _stop.Cancel();
            _game?.Dispose();
            _client?.Dispose();
        }
    }
}
