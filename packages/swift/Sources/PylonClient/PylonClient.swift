import Foundation

#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

/// Configuration for `PylonClient`.
public struct PylonClientConfig: Sendable {
    public var baseURL: URL
    /// App name used to namespace storage keys (matches `pylon codegen`'s
    /// app naming).
    public var appName: String
    public var defaultHeaders: [String: String]
    /// Default `Accept` header. Always includes `application/json`.
    public var transportTimeout: TimeInterval

    public init(
        baseURL: URL,
        appName: String = "default",
        defaultHeaders: [String: String] = [:],
        transportTimeout: TimeInterval = 30
    ) {
        self.baseURL = baseURL
        self.appName = appName
        self.defaultHeaders = defaultHeaders
        self.transportTimeout = transportTimeout
    }
}

/// Core HTTP client. Thread-safe via actor isolation. Holds an auth token,
/// the storage adapter, and the HTTP transport. The sync engine and the
/// realtime client both read the token through this client so a single
/// `setSession(_:)` call updates every subsystem.
public actor PylonClient {
    public let config: PylonClientConfig
    public let storage: PylonStorage
    private let transport: PylonHTTPTransport
    private let encoder: JSONEncoder
    private let decoder: JSONDecoder

    public init(
        config: PylonClientConfig,
        storage: PylonStorage? = nil,
        transport: PylonHTTPTransport? = nil
    ) {
        self.config = config
        self.storage = storage ?? defaultPylonStorage()
        self.transport = transport ?? URLSessionTransport(timeout: config.transportTimeout)
        let enc = JSONEncoder()
        enc.outputFormatting = [.withoutEscapingSlashes]
        self.encoder = enc
        self.decoder = JSONDecoder()
    }

    public init(baseURL: URL, appName: String = "default") {
        self.init(config: PylonClientConfig(baseURL: baseURL, appName: appName))
    }

    // MARK: - Session token

    /// Persist a session token. Picked up automatically on subsequent calls
    /// and by any `SyncEngine` constructed against the same storage.
    public func setSession(token: String) {
        storage.set(StorageKeys.token(appName: config.appName), value: token)
    }

    public func clearSession() {
        storage.remove(StorageKeys.token(appName: config.appName))
    }

    public func currentToken() -> String? {
        storage.get(StorageKeys.token(appName: config.appName))
    }

    // MARK: - Auth endpoints

    /// Begin magic-code sign-in. Server emails the code; caller follows up
    /// with `verifyMagicCode`.
    public func startMagicCode(email: String) async throws {
        let _: EmptyResponse = try await request(.post, "/api/auth/session", body: StartMagicCodeRequest(email: email))
    }

    /// Exchange a magic code for a session token. Token is stored automatically.
    public func verifyMagicCode(email: String, code: String) async throws -> SessionResponse {
        let resp: SessionResponse = try await request(.post, "/api/auth/verify", body: VerifyMagicCodeRequest(email: email, code: code))
        setSession(token: resp.token)
        return resp
    }

    /// Sign in with email + password. Token is stored automatically.
    public func signInWithPassword(email: String, password: String) async throws -> SessionResponse {
        let resp: SessionResponse = try await request(.post, "/api/auth/password/login", body: PasswordSignInRequest(email: email, password: password))
        setSession(token: resp.token)
        return resp
    }

    /// Create an account with email + password. Token is stored
    /// automatically — registering signs you in.
    public func registerWithPassword(email: String, password: String) async throws -> SessionResponse {
        let resp: SessionResponse = try await request(.post, "/api/auth/password/register", body: PasswordSignInRequest(email: email, password: password))
        setSession(token: resp.token)
        return resp
    }

    /// Exchange a Google ID token for a Pylon session.
    public func signInWithGoogle(idToken: String) async throws -> SessionResponse {
        let resp: SessionResponse = try await request(.post, "/api/auth/oauth/google", body: OAuthGoogleRequest(id_token: idToken))
        setSession(token: resp.token)
        return resp
    }

    /// Exchange a GitHub OAuth code for a Pylon session.
    public func signInWithGitHub(code: String) async throws -> SessionResponse {
        let resp: SessionResponse = try await request(.post, "/api/auth/oauth/github", body: OAuthGitHubRequest(code: code))
        setSession(token: resp.token)
        return resp
    }

    /// Resolve the current session — userId, tenantId, roles, isAdmin.
    public func me() async throws -> ResolvedSession {
        try await request(.get, "/api/auth/me")
    }

    public func logout() async throws {
        let _: EmptyResponse = try await request(.post, "/api/auth/logout")
        clearSession()
    }

    /// Periodically re-fetch `/api/auth/me` to keep the session warm.
    /// Returns a handle that cancels the task when released.
    ///
    /// Mirrors `startSessionAutoRefresh` from the React client. The
    /// `intervalSeconds` default of 5 min matches typical session TTLs;
    /// reduce for tighter expiry windows.
    @discardableResult
    public func startSessionAutoRefresh(
        intervalSeconds: TimeInterval = 300,
        onRefresh: (@Sendable (ResolvedSession) -> Void)? = nil,
        onError: (@Sendable (any Error) -> Void)? = nil
    ) -> SessionAutoRefreshHandle {
        let task = Task<Void, Never> { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: UInt64(intervalSeconds * 1_000_000_000))
                guard let self else { return }
                do {
                    let session = try await self.me()
                    onRefresh?(session)
                } catch {
                    onError?(error)
                }
            }
        }
        return SessionAutoRefreshHandle(task: task)
    }

    // MARK: - Entity CRUD

    /// List all rows for an entity. The server wraps list responses in a
    /// `{count, data, limit, offset}` envelope; a bare-array body (older
    /// servers / proxies) is accepted too.
    public func list<T: Decodable>(_ entity: String, as type: T.Type = T.self) async throws -> [T] {
        let envelope: ListEnvelope<T> = try await request(.get, "/api/entities/\(entity)")
        return envelope.rows
    }

    /// Page through an entity using cursor pagination.
    /// - Parameter replication: marks the request as a REPLICATION fetch, so
    ///   the entity's `sync` scope applies. Only the SyncEngine filling the
    ///   local replica should set this — an app paginating the table wants the
    ///   unscoped view, exactly as with `sync: false`. Mirrors the TS engine's
    ///   `sync=1` marker in packages/sync.
    public func listCursor<T: Decodable>(
        _ entity: String,
        after: String? = nil,
        limit: Int = 50,
        replication: Bool = false,
        as type: T.Type = T.self
    ) async throws -> CursorPage<T> {
        var path = "/api/entities/\(entity)/cursor?limit=\(limit)"
        if replication {
            path += "&sync=1"
        }
        if let after, !after.isEmpty {
            path += "&after=\(percentEncode(after))"
        }
        return try await request(.get, path)
    }

    /// Get a single row by ID.
    public func get<T: Decodable>(_ entity: String, id: String, as type: T.Type = T.self) async throws -> T {
        try await request(.get, "/api/entities/\(entity)/\(percentEncode(id))")
    }

    /// Create a row.
    public func create<I: Encodable, O: Decodable>(_ entity: String, _ data: I, as type: O.Type = O.self) async throws -> O {
        try await request(.post, "/api/entities/\(entity)", body: data)
    }

    /// Patch a row.
    public func update<I: Encodable, O: Decodable>(_ entity: String, id: String, _ data: I, as type: O.Type = O.self) async throws -> O {
        try await request(.patch, "/api/entities/\(entity)/\(percentEncode(id))", body: data)
    }

    /// Delete a row.
    public func delete(_ entity: String, id: String) async throws {
        let _: EmptyResponse = try await request(.delete, "/api/entities/\(entity)/\(percentEncode(id))")
    }

    // MARK: - Functions

    /// Invoke a server function and decode its result.
    public func callFn<I: Encodable, O: Decodable>(_ name: String, args: I, as type: O.Type = O.self) async throws -> O {
        try await request(.post, "/api/fn/\(percentEncode(name))", body: args)
    }

    /// Invoke a function and stream its `ctx.stream` output. Yields the
    /// DATA of each SSE frame (multi-line payloads rejoined with `\n`),
    /// finishes when the server sends the terminal `result` frame, and
    /// throws on the terminal `error` frame — parity with the TS
    /// `streamFn`. For raw byte streaming, use `streamFnBytes(_:args:)`.
    ///
    /// Streams are resumable: the server buffers every frame under a
    /// stream id, so a dropped connection reconnects to
    /// `GET /api/fn-streams/<id>` from the last seen cursor and keeps
    /// yielding — including the final result after the handler already
    /// finished. `onStreamId` surfaces the id for cross-launch resumes
    /// via `resumeStream(_:since:)`; `onResult` delivers the terminal
    /// result's raw JSON.
    public func streamFn<I: Encodable>(
        _ name: String,
        args: I,
        resume: Bool = true,
        onStreamId: (@Sendable (String) -> Void)? = nil,
        onResult: (@Sendable (String) -> Void)? = nil
    ) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    let req = try await self.makeRequest(.post, "/api/fn/\(self.percentEncode(name))", body: args, accept: "text/event-stream")
                    let (http, body) = try await self.transport.streamWithResponse(req)
                    if !(200..<300).contains(http.statusCode) {
                        var data = Data()
                        for try await chunk in body { data.append(chunk) }
                        continuation.finish(throwing: self.makeHttpError(status: http.statusCode, data: data))
                        return
                    }
                    let contentType = (http.value(forHTTPHeaderField: "Content-Type") ?? "").lowercased()
                    if !contentType.contains("text/event-stream") {
                        // Non-streaming handler — plain JSON answer.
                        // Deliver it via onResult and finish.
                        var data = Data()
                        for try await chunk in body { data.append(chunk) }
                        if let text = String(data: data, encoding: .utf8) {
                            onResult?(text)
                        }
                        continuation.finish()
                        return
                    }
                    let streamId = http.value(forHTTPHeaderField: "X-Pylon-Stream-Id")
                    if let id = streamId { onStreamId?(id) }
                    let parser = SseParser()
                    var lastSeq: UInt64 = 0
                    var attemptsLeft = 3
                    var currentBody = body
                    while true {
                        var progressed = false
                        var transportError: Error? = nil
                        do {
                            for try await chunk in currentBody {
                                for frame in parser.feed(chunk) {
                                    if let seq = frame.seq { lastSeq = seq }
                                    switch frame.event {
                                    case "result":
                                        onResult?(frame.data)
                                        continuation.finish()
                                        return
                                    case "error":
                                        continuation.finish(throwing: Self.errorFromFrame(frame.data))
                                        return
                                    default:
                                        progressed = true
                                        continuation.yield(frame.data)
                                    }
                                }
                            }
                        } catch {
                            transportError = error
                        }
                        // Ended without a terminal frame — disconnect.
                        guard resume, let id = streamId, attemptsLeft > 0 else {
                            continuation.finish(throwing: transportError ?? PylonError.transport(URLError(.networkConnectionLost)))
                            return
                        }
                        if progressed { attemptsLeft = 3 }
                        attemptsLeft -= 1
                        let resumeReq = try await self.makeRequest(.get, "/api/fn-streams/\(id)?since=\(lastSeq)", body: Optional<Int>.none, accept: "text/event-stream")
                        let (resumedHttp, resumedBody) = try await self.transport.streamWithResponse(resumeReq)
                        if resumedHttp.statusCode == 503, attemptsLeft > 0 {
                            // STREAM_OVERLOADED is transient (Retry-After: 1).
                            for try await _ in resumedBody {}
                            try? await Task.sleep(nanoseconds: 1_000_000_000)
                            continue
                        }
                        if !(200..<300).contains(resumedHttp.statusCode) {
                            var data = Data()
                            for try await chunk in resumedBody { data.append(chunk) }
                            continuation.finish(throwing: self.makeHttpError(status: resumedHttp.statusCode, data: data))
                            return
                        }
                        // Fresh connection — the dead one may have left a
                        // partial frame in the parser; the replay re-sends
                        // that frame whole.
                        parser.reset()
                        currentBody = resumedBody
                    }
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            // A consumer that stops iterating must not leave the
            // producer reconnecting until the stream ends — same
            // lifecycle class as the weak-self Task fixes elsewhere.
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    /// Attach to a buffered fn stream by id — after an app relaunch or
    /// from a different device session. Replays from `since` (0 = the
    /// whole stream, including the final result when the handler already
    /// finished), then live-tails. Same frame semantics as `streamFn`.
    public func resumeStream(
        _ streamId: String,
        since: UInt64 = 0,
        onResult: (@Sendable (String) -> Void)? = nil
    ) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    let parser = SseParser()
                    var lastSeq = since
                    var attemptsLeft = 3
                    while true {
                        let req = try await self.makeRequest(.get, "/api/fn-streams/\(streamId)?since=\(lastSeq)", body: Optional<Int>.none, accept: "text/event-stream")
                        let (http, body) = try await self.transport.streamWithResponse(req)
                        if http.statusCode == 503, attemptsLeft > 0 {
                            attemptsLeft -= 1
                            for try await _ in body {}
                            try? await Task.sleep(nanoseconds: 1_000_000_000)
                            continue
                        }
                        if !(200..<300).contains(http.statusCode) {
                            var data = Data()
                            for try await chunk in body { data.append(chunk) }
                            continuation.finish(throwing: self.makeHttpError(status: http.statusCode, data: data))
                            return
                        }
                        // Drop any partial frame from the previous
                        // connection; the replay re-sends it whole.
                        parser.reset()
                        var progressed = false
                        var transportError: Error? = nil
                        do {
                            for try await chunk in body {
                                for frame in parser.feed(chunk) {
                                    if let seq = frame.seq { lastSeq = seq }
                                    switch frame.event {
                                    case "result":
                                        onResult?(frame.data)
                                        continuation.finish()
                                        return
                                    case "error":
                                        continuation.finish(throwing: Self.errorFromFrame(frame.data))
                                        return
                                    default:
                                        progressed = true
                                        continuation.yield(frame.data)
                                    }
                                }
                            }
                        } catch {
                            transportError = error
                        }
                        if progressed { attemptsLeft = 3 }
                        attemptsLeft -= 1
                        if attemptsLeft < 0 {
                            continuation.finish(throwing: transportError ?? PylonError.transport(URLError(.networkConnectionLost)))
                            return
                        }
                    }
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    private static func errorFromFrame(_ data: String) -> PylonError {
        if let parsed = try? JSONDecoder().decode([String: JSONValue].self, from: Data(data.utf8)),
           case let .string(message)? = parsed["message"] {
            let code: String?
            if case let .string(c)? = parsed["code"] { code = c } else { code = nil }
            return PylonError.http(status: 500, code: code, message: message)
        }
        return PylonError.http(status: 500, code: nil, message: data)
    }

    /// Lower-level streaming helper: yields each Data chunk as it arrives
    /// from the wire. Use when you need raw bytes (e.g. binary streams).
    public func streamFnBytes<I: Encodable>(_ name: String, args: I) -> AsyncThrowingStream<Data, Error> {
        AsyncThrowingStream { continuation in
            Task {
                do {
                    let req = try await self.makeRequest(.post, "/api/fn/\(self.percentEncode(name))", body: args, accept: "application/octet-stream, application/x-ndjson, text/event-stream")
                    try await self.streamBytes(req: req, into: continuation)
                } catch {
                    continuation.finish(throwing: error)
                }
            }
        }
    }

    private func streamBytes(req: URLRequest, into continuation: AsyncThrowingStream<Data, Error>.Continuation) async throws {
        for try await chunk in transport.stream(req) {
            continuation.yield(chunk)
        }
        continuation.finish()
    }

    // MARK: - Aggregate / Search

    /// Aggregate query: count, sum, avg, min, max, countDistinct, groupBy.
    /// `spec` is encoded as JSON exactly as the server expects.
    public func aggregate<I: Encodable, O: Decodable>(_ entity: String, _ spec: I, as type: O.Type = O.self) async throws -> O {
        try await request(.post, "/api/aggregate/\(percentEncode(entity))", body: spec)
    }

    /// Full-text search.
    public func search<I: Encodable, O: Decodable>(_ entity: String, _ spec: I, as type: O.Type = O.self) async throws -> O {
        try await request(.post, "/api/search/\(percentEncode(entity))", body: spec)
    }

    // MARK: - Files

    /// Upload a file. Body is sent as `multipart/form-data` with a single
    /// `file` part.
    public func uploadFile(
        data: Data,
        filename: String,
        contentType: String = "application/octet-stream"
    ) async throws -> FileUploadResponse {
        let boundary = "pylon-\(UUID().uuidString)"
        var body = Data()
        body.append("--\(boundary)\r\n".data(using: .utf8)!)
        body.append("Content-Disposition: form-data; name=\"file\"; filename=\"\(filename)\"\r\n".data(using: .utf8)!)
        body.append("Content-Type: \(contentType)\r\n\r\n".data(using: .utf8)!)
        body.append(data)
        body.append("\r\n--\(boundary)--\r\n".data(using: .utf8)!)

        var req = try await makeRequest(.post, "/api/files/upload", body: Optional<EmptyBody>.none)
        req.setValue("multipart/form-data; boundary=\(boundary)", forHTTPHeaderField: "Content-Type")
        let (respData, http) = try await transport.upload(req, body: body)
        if !(200..<300).contains(http.statusCode) {
            throw makeHttpError(status: http.statusCode, data: respData)
        }
        do {
            return try decoder.decode(FileUploadResponse.self, from: respData)
        } catch {
            throw PylonError.decoding(error)
        }
    }

    /// Download a file's bytes by id. Requires auth.
    public func downloadFile(id: String) async throws -> Data {
        let req = try await makeRequest(.get, "/api/files/\(percentEncode(id))", body: Optional<EmptyBody>.none)
        let (data, http) = try await transport.send(req)
        if !(200..<300).contains(http.statusCode) {
            throw makeHttpError(status: http.statusCode, data: data)
        }
        return data
    }

    // MARK: - Sync (low-level — most callers should use SyncEngine)

    public func syncPull(since: Int64, snapshotAfter: String? = nil) async throws -> PullResponse {
        var path = "/api/sync/pull?since=\(since)"
        if let snapshotAfter, !snapshotAfter.isEmpty {
            // OPAQUE cursor the server already URL-encoded — append verbatim.
            // Re-encoding it double-encodes, the server's parse fails, and it
            // silently restarts the snapshot from page 1: an infinite pull
            // loop for any replica whose snapshot spans >1 page.
            path += "&snapshot_after=\(snapshotAfter)"
        }
        return try await request(.get, path)
    }

    public func syncPush(_ request: PushRequest) async throws -> PushResponse {
        try await self.request(.post, "/api/sync/push", body: request)
    }

    // MARK: - Internal

    enum HTTPVerb: String, Sendable {
        case get = "GET"
        case post = "POST"
        case patch = "PATCH"
        case delete = "DELETE"
        case put = "PUT"
    }

    private struct EmptyBody: Encodable {}
    private struct EmptyResponse: Decodable {}

    func request<O: Decodable>(_ method: HTTPVerb, _ path: String) async throws -> O {
        let req = try await makeRequest(method, path, body: Optional<EmptyBody>.none)
        return try await execute(req)
    }

    func request<I: Encodable, O: Decodable>(_ method: HTTPVerb, _ path: String, body: I) async throws -> O {
        let req = try await makeRequest(method, path, body: body)
        return try await execute(req)
    }

    private func execute<O: Decodable>(_ req: URLRequest) async throws -> O {
        let (data, http) = try await transport.send(req)
        if !(200..<300).contains(http.statusCode) {
            throw makeHttpError(status: http.statusCode, data: data)
        }
        if O.self == EmptyResponse.self {
            return EmptyResponse() as! O
        }
        do {
            return try decoder.decode(O.self, from: data)
        } catch {
            throw PylonError.decoding(error)
        }
    }

    private func makeRequest<I: Encodable>(_ method: HTTPVerb, _ path: String, body: I?, accept: String = "application/json") async throws -> URLRequest {
        let url = config.baseURL.appendingPathComponent(path)
            // appendingPathComponent re-encodes the slashes — rebuild with the
            // raw path to preserve the original.
        let composed = URL(string: path, relativeTo: config.baseURL)?.absoluteURL ?? url
        var req = URLRequest(url: composed)
        req.httpMethod = method.rawValue
        req.setValue(accept, forHTTPHeaderField: "Accept")
        for (k, v) in config.defaultHeaders {
            req.setValue(v, forHTTPHeaderField: k)
        }
        if let token = currentToken() {
            req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        if let body {
            req.setValue("application/json", forHTTPHeaderField: "Content-Type")
            do {
                req.httpBody = try encoder.encode(body)
            } catch {
                throw PylonError.invalidArgument("Failed to encode request body: \(error)")
            }
        }
        return req
    }

    nonisolated func makeHttpError(status: Int, data: Data) -> PylonError {
        // Pylon's router wraps errors as `{"error":{"code","message"}}`
        // (see `json_error` in `crates/router/src/lib.rs`). The previous
        // implementation only decoded `{code, message}` at the top level
        // — Decodable happily ignored the unknown `error` key and
        // returned nil for both, surfacing every 4xx as a bare
        // `PylonError.http(<status>)` with no diagnostic info. Decode
        // the wrapped shape FIRST, then fall back to the top-level
        // shape (some internal endpoints use it), then raw text.
        struct Wrapped: Decodable {
            struct Inner: Decodable { let code: String?; let message: String? }
            let error: Inner?
        }
        struct Flat: Decodable { let code: String?; let message: String? }
        if let wrapped = try? JSONDecoder().decode(Wrapped.self, from: data),
           let inner = wrapped.error,
           inner.code != nil || inner.message != nil
        {
            return .http(status: status, code: inner.code, message: inner.message)
        }
        if let flat = try? JSONDecoder().decode(Flat.self, from: data),
           flat.code != nil || flat.message != nil
        {
            return .http(status: status, code: flat.code, message: flat.message)
        }
        let text = String(data: data, encoding: .utf8)
        return .http(status: status, code: nil, message: text)
    }

    nonisolated func percentEncode(_ s: String) -> String {
        s.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? s
    }
}

/// Cursor-paginated entity response.
public struct CursorPage<T: Decodable & Sendable>: Decodable, Sendable {
    public let data: [T]
    public let next_cursor: String?
    public let has_more: Bool
}

/// Cancellation handle returned by `startSessionAutoRefresh`. Cancels the
/// background task on `cancel()` or when deallocated.
public final class SessionAutoRefreshHandle: @unchecked Sendable {
    private let task: Task<Void, Never>

    init(task: Task<Void, Never>) {
        self.task = task
    }

    public func cancel() { task.cancel() }
    deinit { task.cancel() }
}

// MARK: - SSE frame parser

/// One parsed Server-Sent-Events frame from a Pylon fn stream.
struct SseFrame {
    /// Event type; "message" for plain `data:` frames.
    let event: String
    /// Frame payload — consecutive `data:` lines rejoined with `\n`.
    let data: String
    /// The frame's `id:` (Pylon's per-stream sequence number), if any.
    let seq: UInt64?
}

/// Incremental SSE parser: feed raw body chunks, get complete frames.
/// Comment frames (heartbeats, `: hb`) are dropped. Not thread-safe —
/// use from a single consumer task.
final class SseParser: @unchecked Sendable {
    private var buffer = ""

    /// Drop any partial frame. Call between connections — a resume
    /// replays the interrupted frame whole, so carrying the dead
    /// connection's tail would corrupt and duplicate it.
    func reset() {
        buffer = ""
    }

    func feed(_ chunk: Data) -> [SseFrame] {
        guard let text = String(data: chunk, encoding: .utf8) else { return [] }
        buffer += text
        var frames: [SseFrame] = []
        while let range = buffer.range(of: "\n\n") {
            let raw = String(buffer[buffer.startIndex..<range.lowerBound])
            buffer.removeSubrange(buffer.startIndex..<range.upperBound)
            if let frame = Self.parse(raw) { frames.append(frame) }
        }
        return frames
    }

    private static func parse(_ raw: String) -> SseFrame? {
        var event = "message"
        var dataLines: [String] = []
        var seq: UInt64? = nil
        var sawField = false
        for line in raw.split(separator: "\n", omittingEmptySubsequences: false) {
            if line.hasPrefix(":") { continue } // comment / heartbeat
            if line.hasPrefix("event: ") {
                event = String(line.dropFirst(7))
                sawField = true
            } else if line.hasPrefix("data: ") {
                dataLines.append(String(line.dropFirst(6)))
                sawField = true
            } else if line.hasPrefix("data:") {
                dataLines.append(String(line.dropFirst(5)))
                sawField = true
            } else if line.hasPrefix("id: ") {
                seq = UInt64(line.dropFirst(4).trimmingCharacters(in: .whitespaces))
                sawField = true
            } else if line.hasPrefix("retry:") {
                sawField = true // consumed; reconnect pacing is ours
            }
        }
        // Frames without any data line (the `retry:` prelude, id-only
        // frames) carry nothing to deliver.
        guard sawField, !dataLines.isEmpty else { return nil }
        return SseFrame(event: event, data: dataLines.joined(separator: "\n"), seq: seq)
    }
}
