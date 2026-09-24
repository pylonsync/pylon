import Foundation
import PylonClient

#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

/// One snapshot tick from a shard, decoded to your own `Decodable` state.
public struct ShardSnapshot<State: Decodable & Sendable>: Sendable {
    public let tick: UInt64
    /// The highest `send` sequence number the shard has processed (applied
    /// or rejected) as of this snapshot. Drop local predictions up to it and
    /// replay the rest on top of `state`.
    public let ack: UInt64
    public let state: State
}

/// An input the shard refused.
public struct ShardInputRejection: Sendable, Equatable, Decodable {
    public let clientSeq: UInt64?
    /// `unauthorized`, `rate_limited`, `queue_full`, `invalid`, `stopped`,
    /// or `apply_failed`.
    public let code: String
    public let message: String

    enum CodingKeys: String, CodingKey {
        case clientSeq = "client_seq"
        case code
        case message
    }
}

/// The shard wire protocol, version 2 (`pylon_realtime::wire`). Each server
/// message is an 18-byte header (kind, codec, tick, ack) and a payload.
public enum ShardWire {
    public static let version = 2
    public static let headerLength = 18

    public enum Kind: UInt8, Sendable {
        case snapshot = 1
        case inputRejected = 2
    }

    public enum Codec: UInt8, Sendable {
        case json = 0
        case messagePack = 1
        case bincode = 2
        case custom = 3
    }

    public struct Frame: Sendable {
        public let kind: UInt8
        public let codec: UInt8
        public let tick: UInt64
        public let ack: UInt64
        public let payload: Data
    }

    public enum WireError: Error, Equatable {
        case tooShort(Int)
        case unsupportedCodec(UInt8)
    }

    public static func parse(_ data: Data) throws -> Frame {
        guard data.count >= headerLength else { throw WireError.tooShort(data.count) }
        let bytes = [UInt8](data.prefix(headerLength))
        func u64(_ at: Int) -> UInt64 {
            bytes[at..<(at + 8)].reduce(0) { ($0 << 8) | UInt64($1) }
        }
        return Frame(
            kind: bytes[0],
            codec: bytes[1],
            tick: u64(2),
            ack: u64(10),
            payload: data.subdata(in: (data.startIndex + headerLength)..<data.endIndex)
        )
    }

    /// Decode a payload in JSON or MessagePack.
    public static func decode<T: Decodable>(_ type: T.Type, codec: UInt8, payload: Data) throws -> T {
        switch Codec(rawValue: codec) {
        case .json: return try JSONDecoder().decode(type, from: payload)
        case .messagePack: return try MessagePackDecoder().decode(type, from: payload)
        default: throw WireError.unsupportedCodec(codec)
        }
    }
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
/// Input send: `try await client.send(input)` — wrapped in
/// `{ input, client_seq }`, MessagePack for MessagePack shards, else JSON.
/// Refused inputs: `for await r in client.rejections() { ... }`.
public actor ShardClient<State: Decodable & Sendable, Input: Encodable & Sendable> {
    public let config: ShardClientConfig
    public let shardId: String

    private var task: URLSessionWebSocketTask?
    private var session: URLSession
    private var snapshotContinuation: AsyncStream<ShardSnapshot<State>>.Continuation?
    private var rejectionContinuation: AsyncStream<ShardInputRejection>.Continuation?
    /// The shard's codec, learned from the first frame. Until then inputs go
    /// as JSON text, which every shard accepts.
    private var codec: UInt8?
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

    /// Inputs the shard refused.
    public func rejections() -> AsyncStream<ShardInputRejection> {
        AsyncStream { cont in self.rejectionContinuation = cont }
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
        rejectionContinuation?.finish()
        stateContinuation?.finish()
    }

    /// Send an input, wrapped as `{ "input": <input>, "client_seq": <n> }`
    /// (matches the TS client). A MessagePack shard gets a binary frame;
    /// otherwise the envelope goes as JSON text. Returns the sequence number,
    /// which later snapshots acknowledge.
    @discardableResult
    public func send(_ input: Input) async throws -> UInt64 {
        clientSeq += 1
        let envelope = InputEnvelope(input: input, client_seq: clientSeq)
        guard let task else {
            throw URLError(.notConnectedToInternet)
        }
        if codec == ShardWire.Codec.messagePack.rawValue {
            try await task.send(.data(try MessagePackEncoder().encode(envelope)))
        } else {
            let data = try encoder.encode(envelope)
            guard let text = String(data: data, encoding: .utf8) else { return clientSeq }
            try await task.send(.string(text))
        }
        return clientSeq
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
                    handleFrame(data)
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
            URLQueryItem(name: "v", value: String(ShardWire.version)),
        ]
        return components.url ?? config.baseURL
    }

    private func handleFrame(_ data: Data) {
        guard let frame = try? ShardWire.parse(data) else { return }
        codec = frame.codec
        switch ShardWire.Kind(rawValue: frame.kind) {
        case .snapshot:
            if let state = try? ShardWire.decode(State.self, codec: frame.codec, payload: frame.payload) {
                snapshotContinuation?.yield(ShardSnapshot(tick: frame.tick, ack: frame.ack, state: state))
            }
        case .inputRejected:
            if let rejection = try? ShardWire.decode(
                ShardInputRejection.self, codec: frame.codec, payload: frame.payload)
            {
                rejectionContinuation?.yield(rejection)
            }
        case .none:
            break
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
