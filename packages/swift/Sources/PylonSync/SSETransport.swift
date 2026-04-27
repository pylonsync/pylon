import Foundation

#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

/// Server-Sent Events parser. Pylon's SSE endpoint emits one JSON
/// `ChangeEvent` per `data:` line, separated by blank lines per the
/// SSE spec. Mirror of the parser path in `connectSse()` in the TS
/// engine — we keep it simple (no `event:` typing, no retry hints).
public struct SSEParser {
    private var buffer = ""

    public init() {}

    /// Append a chunk of UTF-8 text and return any complete events.
    /// One event = one or more `data:` lines terminated by a blank line.
    public mutating func feed(_ chunk: String) -> [String] {
        buffer += chunk
        var events: [String] = []
        while let range = buffer.range(of: "\n\n") {
            let frame = buffer[buffer.startIndex..<range.lowerBound]
            buffer.removeSubrange(buffer.startIndex..<range.upperBound)
            // Strip leading "data:" prefix on each line, trim, join with \n.
            let lines: [String] = frame
                .split(separator: "\n", omittingEmptySubsequences: false)
                .compactMap { line -> String? in
                    if let r = line.range(of: "data:") {
                        var s = String(line[r.upperBound...])
                        if s.hasPrefix(" ") { s.removeFirst() }
                        return s
                    }
                    return nil
                }
            guard !lines.isEmpty else { continue }
            events.append(lines.joined(separator: "\n"))
        }
        return events
    }
}

/// HTTP/SSE transport for the sync engine's fallback path. Used when
/// `SyncEngineConfig.transport == .sse`. Connects to `port + 2 /events`
/// (the same convention the TS engine uses) and streams `data:` payloads
/// to the engine.
///
/// Open the stream with `connect()`. The `messages()` async stream yields
/// one `String` per JSON event, mirroring the WebSocket text path so the
/// engine can hand them to the same dispatcher.
public final class SSEStream: @unchecked Sendable {
    private let url: URL
    private let token: String?
    private let session: URLSession
    private var task: URLSessionDataTask?
    private var continuation: AsyncThrowingStream<String, Error>.Continuation?
    private let lock = NSLock()
    private var parser = SSEParser()
    private var delegate: SSEDelegate?

    public init(url: URL, token: String?, session: URLSession? = nil) {
        self.url = url
        self.token = token
        let cfg = URLSessionConfiguration.default
        cfg.timeoutIntervalForRequest = .infinity
        cfg.timeoutIntervalForResource = .infinity
        self.session = session ?? URLSession(configuration: cfg, delegate: nil, delegateQueue: nil)
    }

    public func connect() {
        var req = URLRequest(url: url)
        req.httpMethod = "GET"
        req.setValue("text/event-stream", forHTTPHeaderField: "Accept")
        if let token, !token.isEmpty {
            req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        let delegate = SSEDelegate(onChunk: { [weak self] data in
            guard let self else { return }
            guard let chunk = String(data: data, encoding: .utf8) else { return }
            let events: [String] = {
                self.lock.lock(); defer { self.lock.unlock() }
                return self.parser.feed(chunk)
            }()
            let cont = self.snapshotContinuation()
            for e in events { cont?.yield(e) }
        }, onComplete: { [weak self] error in
            guard let self else { return }
            let cont = self.takeContinuation()
            if let error {
                cont?.finish(throwing: error)
            } else {
                cont?.finish()
            }
        })
        self.delegate = delegate
        // We need a delegate-bound session because the default shared session
        // doesn't deliver streaming chunks via dataTask completion handlers —
        // it buffers the whole body. Build a dedicated session for this stream.
        let streamingSession = URLSession(configuration: session.configuration, delegate: delegate, delegateQueue: nil)
        let task = streamingSession.dataTask(with: req)
        setTask(task)
        task.resume()
    }

    public func messages() -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { cont in
            self.lock.lock()
            self.continuation = cont
            self.lock.unlock()
        }
    }

    public func close() {
        let cont = takeContinuation()
        cont?.finish()
        currentTask()?.cancel()
        setTask(nil)
    }

    private func currentTask() -> URLSessionDataTask? {
        lock.lock(); defer { lock.unlock() }
        return task
    }

    private func setTask(_ task: URLSessionDataTask?) {
        lock.lock(); defer { lock.unlock() }
        self.task = task
    }

    private func snapshotContinuation() -> AsyncThrowingStream<String, Error>.Continuation? {
        lock.lock(); defer { lock.unlock() }
        return continuation
    }

    private func takeContinuation() -> AsyncThrowingStream<String, Error>.Continuation? {
        lock.lock(); defer { lock.unlock() }
        let c = continuation
        continuation = nil
        return c
    }
}

/// `URLSessionDataDelegate` that forwards each incoming chunk to a closure.
/// Necessary because `URLSession.dataTask(with:completionHandler:)` only
/// invokes the handler at completion — we need streaming delivery.
private final class SSEDelegate: NSObject, URLSessionDataDelegate {
    let onChunk: (Data) -> Void
    let onComplete: ((any Error)?) -> Void

    init(onChunk: @escaping (Data) -> Void, onComplete: @escaping ((any Error)?) -> Void) {
        self.onChunk = onChunk
        self.onComplete = onComplete
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        onChunk(data)
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: (any Error)?) {
        onComplete(error)
    }
}
