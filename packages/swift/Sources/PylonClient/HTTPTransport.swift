import Foundation

#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

/// Pluggable HTTP transport. Swap in a fake for tests; the default delegates
/// to `URLSession`. Kept narrow so a `URLProtocol` subclass or a mock
/// transport doesn't need to subclass the entire client.
public protocol PylonHTTPTransport: Sendable {
    func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse)
    func upload(_ request: URLRequest, body: Data) async throws -> (Data, HTTPURLResponse)

    /// Stream the response body. Default implementation buffers the whole
    /// response and yields it as a single chunk — fine for mocks and for
    /// the Linux path. The `URLSessionTransport` impl below overrides this
    /// on iOS 15+/macOS 12+ to deliver chunks as they arrive.
    func stream(_ request: URLRequest) -> AsyncThrowingStream<Data, Error>

    /// Stream variant that also surfaces the response head, so callers can
    /// read headers (`X-Pylon-Stream-Id`, `Content-Type`) before consuming
    /// the body. Default implementation buffers like `stream`.
    func streamWithResponse(
        _ request: URLRequest
    ) async throws -> (HTTPURLResponse, AsyncThrowingStream<Data, Error>)
}

public extension PylonHTTPTransport {
    func stream(_ request: URLRequest) -> AsyncThrowingStream<Data, Error> {
        AsyncThrowingStream { continuation in
            Task {
                do {
                    let (data, http) = try await self.send(request)
                    if !(200..<300).contains(http.statusCode) {
                        continuation.finish(throwing: PylonError.http(status: http.statusCode, code: nil, message: String(data: data, encoding: .utf8)))
                        return
                    }
                    continuation.yield(data)
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }
        }
    }

    func streamWithResponse(
        _ request: URLRequest
    ) async throws -> (HTTPURLResponse, AsyncThrowingStream<Data, Error>) {
        let (data, http) = try await self.send(request)
        let body = AsyncThrowingStream<Data, Error> { continuation in
            if !data.isEmpty { continuation.yield(data) }
            continuation.finish()
        }
        return (http, body)
    }
}

/// Default transport. Uses a single `URLSession` configured with a 30s
/// resource timeout and HTTP/2 enabled by default.
public final class URLSessionTransport: PylonHTTPTransport, @unchecked Sendable {
    private let session: URLSession

    public init(session: URLSession = .shared) {
        self.session = session
    }

    public convenience init(timeout: TimeInterval) {
        let cfg = URLSessionConfiguration.default
        cfg.timeoutIntervalForRequest = timeout
        cfg.timeoutIntervalForResource = timeout * 2
        self.init(session: URLSession(configuration: cfg))
    }

    public func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        try await asHttp(try await dataTask(for: request))
    }

    public func upload(_ request: URLRequest, body: Data) async throws -> (Data, HTTPURLResponse) {
        try await asHttp(try await uploadTask(for: request, body: body))
    }

    public func stream(_ request: URLRequest) -> AsyncThrowingStream<Data, Error> {
        #if !canImport(FoundationNetworking)
        if #available(iOS 15.0, macOS 12.0, tvOS 15.0, watchOS 8.0, *) {
            return AsyncThrowingStream { continuation in
                Task {
                    do {
                        let (bytes, response) = try await session.bytes(for: request)
                        if let http = response as? HTTPURLResponse, !(200..<300).contains(http.statusCode) {
                            var body = Data()
                            for try await b in bytes { body.append(b) }
                            continuation.finish(throwing: PylonError.http(status: http.statusCode, code: nil, message: String(data: body, encoding: .utf8)))
                            return
                        }
                        var buffer = Data()
                        buffer.reserveCapacity(4096)
                        for try await b in bytes {
                            buffer.append(b)
                            if buffer.count >= 4096 {
                                continuation.yield(buffer)
                                buffer.removeAll(keepingCapacity: true)
                            }
                        }
                        if !buffer.isEmpty { continuation.yield(buffer) }
                        continuation.finish()
                    } catch {
                        continuation.finish(throwing: error)
                    }
                }
            }
        }
        #endif
        // Fallback for Linux / older Apple OSes: rely on the protocol-level
        // default implementation (single-chunk send).
        return AsyncThrowingStream { continuation in
            Task {
                do {
                    let (data, http) = try await self.send(request)
                    if !(200..<300).contains(http.statusCode) {
                        continuation.finish(throwing: PylonError.http(status: http.statusCode, code: nil, message: String(data: data, encoding: .utf8)))
                        return
                    }
                    continuation.yield(data)
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }
        }
    }

    public func streamWithResponse(
        _ request: URLRequest
    ) async throws -> (HTTPURLResponse, AsyncThrowingStream<Data, Error>) {
        #if !canImport(FoundationNetworking)
        if #available(iOS 15.0, macOS 12.0, tvOS 15.0, watchOS 8.0, *) {
            let (bytes, response) = try await session.bytes(for: request)
            guard let http = response as? HTTPURLResponse else {
                throw PylonError.transport(URLError(.badServerResponse))
            }
            let body = AsyncThrowingStream<Data, Error> { continuation in
                Task {
                    do {
                        // Flush on every newline (SSE lines are short) or
                        // at 4KB, whichever first. A pure size threshold
                        // withheld live tokens until ~4KB accumulated —
                        // a heartbeat-only tail would never flush at all.
                        var buffer = Data()
                        buffer.reserveCapacity(4096)
                        for try await b in bytes {
                            buffer.append(b)
                            if b == 0x0A || buffer.count >= 4096 {
                                continuation.yield(buffer)
                                buffer.removeAll(keepingCapacity: true)
                            }
                        }
                        if !buffer.isEmpty { continuation.yield(buffer) }
                        continuation.finish()
                    } catch {
                        continuation.finish(throwing: error)
                    }
                }
            }
            return (http, body)
        }
        #endif
        // Linux / older Apple OSes: buffered fallback.
        let (data, http) = try await self.send(request)
        let body = AsyncThrowingStream<Data, Error> { continuation in
            if !data.isEmpty { continuation.yield(data) }
            continuation.finish()
        }
        return (http, body)
    }

    private func dataTask(for request: URLRequest) async throws -> (Data, URLResponse) {
        #if canImport(FoundationNetworking)
        // Linux's URLSession doesn't support async/await out of the box —
        // fall back to a continuation around the completion-handler API.
        return try await withCheckedThrowingContinuation { cont in
            let task = session.dataTask(with: request) { data, response, error in
                if let error { cont.resume(throwing: error); return }
                guard let data, let response else {
                    cont.resume(throwing: PylonError.transport(URLError(.badServerResponse)))
                    return
                }
                cont.resume(returning: (data, response))
            }
            task.resume()
        }
        #else
        return try await session.data(for: request)
        #endif
    }

    private func uploadTask(for request: URLRequest, body: Data) async throws -> (Data, URLResponse) {
        #if canImport(FoundationNetworking)
        return try await withCheckedThrowingContinuation { cont in
            let task = session.uploadTask(with: request, from: body) { data, response, error in
                if let error { cont.resume(throwing: error); return }
                guard let data, let response else {
                    cont.resume(throwing: PylonError.transport(URLError(.badServerResponse)))
                    return
                }
                cont.resume(returning: (data, response))
            }
            task.resume()
        }
        #else
        return try await session.upload(for: request, from: body)
        #endif
    }

    private func asHttp(_ pair: (Data, URLResponse)) async throws -> (Data, HTTPURLResponse) {
        guard let http = pair.1 as? HTTPURLResponse else {
            throw PylonError.transport(URLError(.badServerResponse))
        }
        return (pair.0, http)
    }
}
