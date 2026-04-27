import Foundation
@testable import PylonClient

/// Test transport that captures the request and replies with a canned response.
/// Avoids a URLProtocol subclass so the same harness works on Linux's
/// FoundationNetworking.
public final class MockTransport: PylonHTTPTransport, @unchecked Sendable {
    public typealias Handler = @Sendable (URLRequest) async throws -> (Int, Data)

    private let lock = NSLock()
    private var handler: Handler
    public private(set) var requests: [URLRequest] = []
    public private(set) var bodies: [Data] = []

    public init(_ handler: @escaping Handler = { _ in (200, Data()) }) {
        self.handler = handler
    }

    public func setHandler(_ handler: @escaping Handler) {
        lock.lock(); defer { lock.unlock() }
        self.handler = handler
    }

    public func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        lock.lock()
        requests.append(request)
        bodies.append(request.httpBody ?? Data())
        let h = handler
        lock.unlock()
        let (status, data) = try await h(request)
        let resp = HTTPURLResponse(
            url: request.url ?? URL(string: "http://invalid")!,
            statusCode: status,
            httpVersion: "HTTP/1.1",
            headerFields: ["Content-Type": "application/json"]
        )!
        return (data, resp)
    }

    public func upload(_ request: URLRequest, body: Data) async throws -> (Data, HTTPURLResponse) {
        lock.lock()
        var copy = request
        copy.httpBody = body
        requests.append(copy)
        bodies.append(body)
        let h = handler
        lock.unlock()
        let (status, data) = try await h(copy)
        let resp = HTTPURLResponse(
            url: request.url ?? URL(string: "http://invalid")!,
            statusCode: status,
            httpVersion: "HTTP/1.1",
            headerFields: ["Content-Type": "application/json"]
        )!
        return (data, resp)
    }

    public func lastRequest() -> URLRequest? {
        lock.lock(); defer { lock.unlock() }
        return requests.last
    }

    public func lastBody() -> Data? {
        lock.lock(); defer { lock.unlock() }
        return bodies.last
    }
}

public func jsonResponse(_ object: Any, status: Int = 200) throws -> (Int, Data) {
    let data = try JSONSerialization.data(withJSONObject: object)
    return (status, data)
}
