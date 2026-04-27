import Foundation

#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

/// One inbound message received over the WebSocket. The sync engine
/// dispatches text payloads as JSON change events / control frames, and
/// routes binary frames to registered consumers (e.g. Loro CRDT decoders).
public enum WSMessage: Sendable {
    case text(String)
    case binary(Data)
}

/// WebSocket transport abstraction. Default impl uses
/// `URLSessionWebSocketTask`; tests can plug a fake.
public protocol PylonWebSocket: Sendable {
    /// Open the connection. Throws on transport failure (DNS, TLS, refused).
    func connect() async throws
    func send(text: String) async throws
    func send(binary: Data) async throws
    /// Async stream of incoming messages. Ends when the socket closes.
    func messages() -> AsyncThrowingStream<WSMessage, Error>
    func close()
}

/// `URLSessionWebSocketTask`-backed implementation.
///
/// The pylon server reads bearer tokens from the `Sec-WebSocket-Protocol`
/// header (subprotocol `bearer.<percent-encoded-token>`) when the request
/// can't carry an `Authorization` header. We always pass the subprotocol
/// when a token is configured — that matches the TS engine and keeps the
/// path identical for browsers and natives.
public final class URLSessionWebSocket: PylonWebSocket, @unchecked Sendable {
    private let url: URL
    private let token: String?
    private let session: URLSession
    private var task: URLSessionWebSocketTask?
    private var continuation: AsyncThrowingStream<WSMessage, Error>.Continuation?
    private let lock = NSLock()

    public init(url: URL, token: String?, session: URLSession = .shared) {
        self.url = url
        self.token = token
        self.session = session
    }

    public func connect() async throws {
        var protocols: [String]? = nil
        if let token, !token.isEmpty {
            let escaped = token.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) ?? token
            protocols = ["bearer.\(escaped)"]
        }
        var request = URLRequest(url: url)
        if let protocols {
            request.setValue(protocols.joined(separator: ", "), forHTTPHeaderField: "Sec-WebSocket-Protocol")
        }
        let task = session.webSocketTask(with: request)
        setTask(task)
        task.resume()
        // Kick off the receive loop. URLSessionWebSocketTask delivers one
        // message per receive() call, so we recursively re-issue.
        startReceiveLoop()
    }

    public func send(text: String) async throws {
        guard let task = currentTask() else {
            throw URLError(.notConnectedToInternet)
        }
        try await task.send(.string(text))
    }

    public func send(binary: Data) async throws {
        guard let task = currentTask() else {
            throw URLError(.notConnectedToInternet)
        }
        try await task.send(.data(binary))
    }

    public func messages() -> AsyncThrowingStream<WSMessage, Error> {
        AsyncThrowingStream { cont in
            lock.lock()
            self.continuation = cont
            lock.unlock()
        }
    }

    public func close() {
        lock.lock()
        let task = self.task
        let cont = self.continuation
        self.task = nil
        self.continuation = nil
        lock.unlock()
        task?.cancel(with: .normalClosure, reason: nil)
        cont?.finish()
    }

    private func currentTask() -> URLSessionWebSocketTask? {
        lock.lock(); defer { lock.unlock() }
        return task
    }

    private func setTask(_ task: URLSessionWebSocketTask) {
        lock.lock(); defer { lock.unlock() }
        self.task = task
    }

    private func startReceiveLoop() {
        guard let task = currentTask() else { return }
        Task { [weak self] in
            while let self {
                do {
                    let message = try await task.receive()
                    let cont = self.snapshotContinuation()
                    switch message {
                    case .string(let s):
                        cont?.yield(.text(s))
                    case .data(let d):
                        cont?.yield(.binary(d))
                    @unknown default:
                        break
                    }
                } catch {
                    let cont = self.takeContinuation()
                    cont?.finish(throwing: error)
                    return
                }
            }
        }
    }

    private func snapshotContinuation() -> AsyncThrowingStream<WSMessage, Error>.Continuation? {
        lock.lock(); defer { lock.unlock() }
        return continuation
    }

    private func takeContinuation() -> AsyncThrowingStream<WSMessage, Error>.Continuation? {
        lock.lock(); defer { lock.unlock() }
        let c = continuation
        continuation = nil
        return c
    }
}

// MARK: - Backoff

/// Exponential backoff with full jitter. Mirrors the algorithm in the TS
/// `SyncEngine.computeBackoff`. Returns delay in seconds.
public func computeBackoff(attempts: Int, baseDelay: TimeInterval = 1.0, maxDelay: TimeInterval = 30.0) -> TimeInterval {
    let attempt = max(1, attempts)
    let exp = min(maxDelay, baseDelay * pow(2.0, Double(attempt - 1)))
    return Double.random(in: 0...exp)
}
