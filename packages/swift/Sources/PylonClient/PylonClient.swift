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
        setSession(token: resp.session_token)
        return resp
    }

    /// Sign in with email + password. Token is stored automatically.
    public func signInWithPassword(email: String, password: String) async throws -> SessionResponse {
        let resp: SessionResponse = try await request(.post, "/api/auth/password", body: PasswordSignInRequest(email: email, password: password))
        setSession(token: resp.session_token)
        return resp
    }

    /// Exchange a Google ID token for a Pylon session.
    public func signInWithGoogle(idToken: String) async throws -> SessionResponse {
        let resp: SessionResponse = try await request(.post, "/api/auth/oauth/google", body: OAuthGoogleRequest(id_token: idToken))
        setSession(token: resp.session_token)
        return resp
    }

    /// Exchange a GitHub OAuth code for a Pylon session.
    public func signInWithGitHub(code: String) async throws -> SessionResponse {
        let resp: SessionResponse = try await request(.post, "/api/auth/oauth/github", body: OAuthGitHubRequest(code: code))
        setSession(token: resp.session_token)
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

    /// List all rows for an entity.
    public func list<T: Decodable>(_ entity: String, as type: T.Type = T.self) async throws -> [T] {
        try await request(.get, "/api/entities/\(entity)")
    }

    /// Page through an entity using cursor pagination.
    public func listCursor<T: Decodable>(
        _ entity: String,
        after: String? = nil,
        limit: Int = 50,
        as type: T.Type = T.self
    ) async throws -> CursorPage<T> {
        var path = "/api/entities/\(entity)/cursor?limit=\(limit)"
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

    /// Invoke a function and stream the body as it arrives. Useful for AI
    /// chat / live data. Each yielded chunk is one line (delimited by
    /// `\n`) — works for NDJSON and SSE-flavored function outputs. For
    /// raw byte streaming, use `streamFnBytes(_:args:)`.
    public func streamFn<I: Encodable>(_ name: String, args: I) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { continuation in
            Task {
                do {
                    let req = try await self.makeRequest(.post, "/api/fn/\(self.percentEncode(name))", body: args, accept: "text/event-stream, application/x-ndjson, application/octet-stream")
                    try await self.streamLines(req: req, into: continuation)
                } catch {
                    continuation.finish(throwing: error)
                }
            }
        }
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

    private func streamLines(req: URLRequest, into continuation: AsyncThrowingStream<String, Error>.Continuation) async throws {
        var pending = ""
        for try await chunk in transport.stream(req) {
            guard let text = String(data: chunk, encoding: .utf8) else { continue }
            pending += text
            while let nl = pending.firstIndex(of: "\n") {
                let line = String(pending[pending.startIndex..<nl])
                pending.removeSubrange(pending.startIndex...nl)
                continuation.yield(line)
            }
        }
        if !pending.isEmpty {
            continuation.yield(pending)
        }
        continuation.finish()
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

    public func syncPull(since: Int64) async throws -> PullResponse {
        try await request(.get, "/api/sync/pull?since=\(since)")
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
        // Try to parse `{ code, message }` from the body.
        struct ErrorBody: Decodable { let code: String?; let message: String? }
        if let body = try? JSONDecoder().decode(ErrorBody.self, from: data) {
            return .http(status: status, code: body.code, message: body.message)
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
