import Foundation
import PylonClient

#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

/// One snapshot tick from a shard. The server frames each broadcast as
/// `[tick: u64 BE | snapshot: JSON bytes]`. Decode the snapshot to your
/// own `Decodable` shard state.
public struct ShardSnapshot<State: Decodable & Sendable>: Sendable {
    public let tick: UInt64
    public let state: State
}

public struct ShardClientConfig: Sendable {
    public var baseURL: URL
    /// WebSocket port. `pylon dev` exposes shards on `port + 3`.
    public var wsPort: Int?
    /// Override the full WebSocket URL (overrides `baseURL` + `wsPort`).
    public var wsURL: URL?
    public var subscriberId: String
    public var token: String?
    public var autoReconnect: Bool
    public var reconnectBaseDelay: TimeInterval

    public init(
        baseURL: URL,
        subscriberId: String,
        token: String? = nil,
        wsPort: Int? = nil,
        wsURL: URL? = nil,
        autoReconnect: Bool = true,
        reconnectBaseDelay: TimeInterval = 0.5
    ) {
        self.baseURL = baseURL
        self.subscriberId = subscriberId
        self.token = token
        self.wsPort = wsPort
        self.wsURL = wsURL
        self.autoReconnect = autoReconnect
        self.reconnectBaseDelay = reconnectBaseDelay
    }
}

/// Realtime shard client. Connects to a tick-driven simulation, decodes
/// snapshots, and ships inputs back. Mirrors the React `useShard` hook
/// in `packages/react/src/useShard.ts`.
///
/// Snapshot stream: `for await snap in client.snapshots() { ... }`
/// Input send: `try await client.send(input)` — encoded as JSON, wrapped
/// in `{ input, client_seq }`.
public actor ShardClient<State: Decodable & Sendable, Input: Encodable & Sendable> {
    public let config: ShardClientConfig
    public let shardId: String

    private var task: URLSessionWebSocketTask?
    private var session: URLSession
    private var snapshotContinuation: AsyncStream<ShardSnapshot<State>>.Continuation?
    private var stateContinuation: AsyncStream<ConnectionState>.Continuation?
    private var clientSeq: UInt64 = 0
    private var reconnectAttempts = 0
    private var running = false

    public enum ConnectionState: Sendable {
        case disconnected
        case connecting
        case connected
        case failed(String)
    }

    private let decoder: JSONDecoder
    private let encoder: JSONEncoder

    public init(
        shardId: String,
        config: ShardClientConfig,
        session: URLSession = .shared
    ) {
        self.shardId = shardId
        self.config = config
        self.session = session
        self.decoder = JSONDecoder()
        self.encoder = JSONEncoder()
    }

    public func snapshots() -> AsyncStream<ShardSnapshot<State>> {
        AsyncStream { cont in self.snapshotContinuation = cont }
    }

    public func connectionStates() -> AsyncStream<ConnectionState> {
        AsyncStream { cont in self.stateContinuation = cont }
    }

    /// Connect (or reconnect) the WebSocket. The snapshot stream stays
    /// alive across reconnects.
    public func connect() async {
        guard !running else { return }
        running = true
        await openSocket()
    }

    public func close() {
        running = false
        task?.cancel(with: .normalClosure, reason: nil)
        task = nil
        snapshotContinuation?.finish()
        stateContinuation?.finish()
    }

    /// Send an input. The framing wraps your `Input` value as
    /// `{ "input": <input>, "client_seq": <n> }` (matches the TS hook).
    public func send(_ input: Input) async throws {
        clientSeq += 1
        let envelope = InputEnvelope(input: input, client_seq: clientSeq)
        let data = try encoder.encode(envelope)
        guard let text = String(data: data, encoding: .utf8) else { return }
        guard let task else {
            throw URLError(.notConnectedToInternet)
        }
        try await task.send(.string(text))
    }

    // MARK: - Internals

    private func openSocket() async {
        guard running else { return }
        stateContinuation?.yield(.connecting)
        let url = deriveURL()
        var protocols: [String] = []
        if let token = config.token, !token.isEmpty {
            let escaped = token.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) ?? token
            protocols.append("bearer.\(escaped)")
        }
        var req = URLRequest(url: url)
        if !protocols.isEmpty {
            req.setValue(protocols.joined(separator: ", "), forHTTPHeaderField: "Sec-WebSocket-Protocol")
        }
        let task = session.webSocketTask(with: req)
        self.task = task
        task.resume()
        stateContinuation?.yield(.connected)
        reconnectAttempts = 0
        await receiveLoop(task: task)
    }

    private func receiveLoop(task: URLSessionWebSocketTask) async {
        while running {
            do {
                let message = try await task.receive()
                switch message {
                case .data(let data):
                    if let snap = decodeSnapshot(data) {
                        snapshotContinuation?.yield(snap)
                    }
                case .string:
                    // Servers may emit JSON control messages; ignore for now.
                    break
                @unknown default:
                    break
                }
            } catch {
                stateContinuation?.yield(.failed("\(error)"))
                if config.autoReconnect, running {
                    await scheduleReconnect()
                }
                return
            }
        }
    }

    private func scheduleReconnect() async {
        reconnectAttempts += 1
        let delay = exponentialBackoff(attempts: reconnectAttempts, baseDelay: config.reconnectBaseDelay, maxDelay: 10.0)
        try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
        await openSocket()
    }

    private func deriveURL() -> URL {
        if let override = config.wsURL { return override }
        var components = URLComponents(url: config.baseURL, resolvingAgainstBaseURL: false)!
        let isHttps = components.scheme == "https"
        components.scheme = isHttps ? "wss" : "ws"
        let basePort = components.port ?? (isHttps ? 443 : 80)
        components.port = config.wsPort ?? (basePort + 3)
        components.path = "/"
        components.queryItems = [
            URLQueryItem(name: "shard", value: shardId),
            URLQueryItem(name: "sid", value: config.subscriberId),
        ]
        return components.url ?? config.baseURL
    }

    private func decodeSnapshot(_ data: Data) -> ShardSnapshot<State>? {
        guard data.count >= 8 else { return nil }
        let base = data.startIndex
        var tick: UInt64 = 0
        for i in 0..<8 {
            tick = (tick << 8) | UInt64(data[base + i])
        }
        let payload = data.subdata(in: (base + 8)..<data.endIndex)
        do {
            let state = try decoder.decode(State.self, from: payload)
            return ShardSnapshot(tick: tick, state: state)
        } catch {
            return nil
        }
    }

    private struct InputEnvelope: Encodable {
        let input: Input
        let client_seq: UInt64
    }
}

/// Exponential backoff with full jitter — same algorithm as the sync engine
/// uses, kept local so PylonRealtime doesn't depend on PylonSync.
public func exponentialBackoff(attempts: Int, baseDelay: TimeInterval, maxDelay: TimeInterval) -> TimeInterval {
    let attempt = max(1, attempts)
    let exp = min(maxDelay, baseDelay * pow(2.0, Double(attempt - 1)))
    return Double.random(in: 0...exp)
}
