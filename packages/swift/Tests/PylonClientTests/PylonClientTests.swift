import XCTest
@testable import PylonClient

final class PylonClientTests: XCTestCase {
    func makeClient(transport: MockTransport, storage: PylonStorage = MemoryStorage()) -> PylonClient {
        let cfg = PylonClientConfig(baseURL: URL(string: "http://localhost:4321")!)
        return PylonClient(config: cfg, storage: storage, transport: transport)
    }

    // NOTE: every session-issuing route on the server returns the key
    // `token` (crates/router/src/routes/auth.rs) — these mocks mirror the
    // REAL wire shape. An earlier version of this test mocked
    // `session_token`, which encoded a decode bug instead of catching it.
    func testMagicCodeFlowStoresToken() async throws {
        let transport = MockTransport()
        transport.setHandler { req in
            if req.url?.path == "/api/auth/session" { return (200, Data("{}".utf8)) }
            if req.url?.path == "/api/auth/verify" {
                let body: [String: Any] = ["token": "tok_abc", "user_id": "u1", "expires_at": 1785864905]
                return try jsonResponse(body)
            }
            XCTFail("Unexpected path: \(req.url?.path ?? "")")
            return (404, Data())
        }
        let storage = MemoryStorage()
        let client = makeClient(transport: transport, storage: storage)
        try await client.startMagicCode(email: "alice@example.com")
        let resp = try await client.verifyMagicCode(email: "alice@example.com", code: "123456")
        XCTAssertEqual(resp.token, "tok_abc")
        XCTAssertEqual(storage.get(StorageKeys.token()), "tok_abc")
    }

    func testPasswordLoginHitsLoginRouteAndStoresToken() async throws {
        let transport = MockTransport()
        transport.setHandler { req in
            // The route is /api/auth/password/login — NOT /api/auth/password,
            // which the server 404s.
            if req.url?.path == "/api/auth/password/login", req.httpMethod == "POST" {
                let body: [String: Any] = ["token": "tok_pw", "user_id": "u2", "expires_at": 1785864905]
                return try jsonResponse(body)
            }
            XCTFail("Unexpected path: \(req.url?.path ?? "")")
            return (404, Data())
        }
        let storage = MemoryStorage()
        let client = makeClient(transport: transport, storage: storage)
        let resp = try await client.signInWithPassword(email: "a@b.c", password: "hunter2hunter2")
        XCTAssertEqual(resp.token, "tok_pw")
        XCTAssertEqual(resp.user_id, "u2")
        XCTAssertEqual(storage.get(StorageKeys.token()), "tok_pw")
    }

    func testPasswordRegisterHitsRegisterRouteAndStoresToken() async throws {
        let transport = MockTransport()
        transport.setHandler { req in
            if req.url?.path == "/api/auth/password/register", req.httpMethod == "POST" {
                // Real register responses also carry extra keys the client
                // must tolerate (dev_code, verification_email_sent).
                let body: [String: Any] = [
                    "token": "tok_new", "user_id": "u3", "expires_at": 1785864905,
                    "dev_code": "411527", "verification_email_sent": true,
                ]
                return try jsonResponse(body)
            }
            XCTFail("Unexpected path: \(req.url?.path ?? "")")
            return (404, Data())
        }
        let storage = MemoryStorage()
        let client = makeClient(transport: transport, storage: storage)
        let resp = try await client.registerWithPassword(email: "new@b.c", password: "hunter2hunter2")
        XCTAssertEqual(resp.token, "tok_new")
        XCTAssertEqual(storage.get(StorageKeys.token()), "tok_new")
    }

    func testAuthHeaderSentAfterSetSession() async throws {
        let transport = MockTransport()
        transport.setHandler { _ in
            let body: [String: Any] = [
                "user_id": "u1", "tenant_id": NSNull(), "is_admin": false, "roles": []
            ]
            return try jsonResponse(body)
        }
        let client = makeClient(transport: transport)
        await client.setSession(token: "tok_xyz")
        _ = try await client.me()
        XCTAssertEqual(transport.lastRequest()?.value(forHTTPHeaderField: "Authorization"), "Bearer tok_xyz")
    }

    func testEntityCRUDRoundTrip() async throws {
        struct Todo: Codable, Equatable {
            let id: String
            let title: String
            let done: Bool
        }
        let transport = MockTransport()
        transport.setHandler { req in
            switch (req.httpMethod, req.url?.path) {
            case ("GET", "/api/entities/Todo"):
                return try jsonResponse([
                    ["id": "t1", "title": "ship swift sdk", "done": false]
                ])
            case ("POST", "/api/entities/Todo"):
                return try jsonResponse(["id": "t2", "title": "review", "done": false])
            case ("DELETE", "/api/entities/Todo/t2"):
                return (200, Data("{}".utf8))
            default:
                XCTFail("Unexpected: \(req.httpMethod ?? "?") \(req.url?.path ?? "?")")
                return (404, Data())
            }
        }
        let client = makeClient(transport: transport)
        let list: [Todo] = try await client.list("Todo")
        XCTAssertEqual(list.first?.title, "ship swift sdk")

        struct NewTodo: Encodable { let title: String; let done: Bool }
        let created: Todo = try await client.create("Todo", NewTodo(title: "review", done: false))
        XCTAssertEqual(created.id, "t2")

        try await client.delete("Todo", id: "t2")
    }

    func testHttpErrorSurfacedWithCodeAndMessage() async throws {
        let transport = MockTransport()
        transport.setHandler { _ in
            let body: [String: Any] = ["code": "RATE_LIMITED", "message": "slow down"]
            return try jsonResponse(body, status: 429)
        }
        let client = makeClient(transport: transport)
        do {
            let _: ResolvedSession = try await client.me()
            XCTFail("Expected throw")
        } catch let error as PylonError {
            XCTAssertEqual(error.httpStatus, 429)
            XCTAssertEqual(error.code, "RATE_LIMITED")
        }
    }

    func testFileUploadIsMultipart() async throws {
        let transport = MockTransport()
        transport.setHandler { req in
            let ct = req.value(forHTTPHeaderField: "Content-Type") ?? ""
            XCTAssertTrue(ct.starts(with: "multipart/form-data; boundary=pylon-"))
            return try jsonResponse(["id": "file_xyz", "url": "/api/files/file_xyz", "size": 5])
        }
        let client = makeClient(transport: transport)
        let resp = try await client.uploadFile(data: Data("hello".utf8), filename: "x.txt", contentType: "text/plain")
        XCTAssertEqual(resp.id, "file_xyz")
    }

    func testStorageKeysRespectAppName() {
        XCTAssertEqual(StorageKeys.token(), "pylon_token")
        XCTAssertEqual(StorageKeys.token(appName: "myapp"), "pylon:myapp:token")
    }

    func testJSONValueRoundTrip() throws {
        let original: JSONValue = [
            "id": "u1",
            "active": true,
            "score": 3.14,
            "tags": ["red", "green"],
            "meta": ["count": 42]
        ]
        let encoder = JSONEncoder()
        encoder.outputFormatting = .sortedKeys
        let data = try encoder.encode(original)
        let decoded = try JSONDecoder().decode(JSONValue.self, from: data)
        XCTAssertEqual(decoded["id"].stringValue, "u1")
        XCTAssertEqual(decoded["active"].boolValue, true)
        XCTAssertEqual(decoded["score"].doubleValue, 3.14)
        XCTAssertEqual(decoded["tags"][0].stringValue, "red")
        XCTAssertEqual(decoded["meta"]["count"].intValue, 42)
    }
}
