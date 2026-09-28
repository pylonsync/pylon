import XCTest
import PylonClient
@testable import PylonSync

/// Live change frames are group-committed to SQLite. Parity with
/// `packages/sync/src/live-batch-persist.test.ts`.
///
/// Each WS frame used to await its own row write and a separate cursor
/// write (two autocommit transactions) before the next frame could apply.
/// Frames now apply to memory at once and a single writer puts everything
/// queued while a write is in flight into one transaction with the cursor.
final class LiveBatchPersistTests: XCTestCase {

    /// Delegates to SQLite and counts the writes the engine asks for.
    final class CountingPersistence: SyncPersistence, @unchecked Sendable {
        let inner: SQLitePersistence
        private let lock = NSLock()
        private var _batches = 0
        private var _rowWrites = 0
        var batches: Int { lock.lock(); defer { lock.unlock() }; return _batches }
        var rowWrites: Int { lock.lock(); defer { lock.unlock() }; return _rowWrites }
        var failNextBatch = false

        init(path: String) throws { inner = try SQLitePersistence(path: path) }

        func loadAllRows() async throws -> [String: [Row]] { try await inner.loadAllRows() }
        func loadCursor() async throws -> SyncCursor? { try await inner.loadCursor() }
        func saveCursor(_ cursor: SyncCursor) async throws { try await inner.saveCursor(cursor) }
        private func locked<T>(_ body: () -> T) -> T {
            lock.lock(); defer { lock.unlock() }
            return body()
        }
        func persist(_ change: ChangeEvent) async throws {
            locked { _rowWrites += 1 }
            try await inner.persist(change)
        }
        func clearRows() async throws { try await inner.clearRows() }
        func persistBatch(_ changes: [ChangeEvent], cursor: SyncCursor?) async throws {
            let fail: Bool = locked {
                _batches += 1
                let f = failNextBatch
                failNextBatch = false
                return f
            }
            if fail { throw PylonError.http(status: 507, code: "DISK_FULL", message: nil) }
            try await inner.persistBatch(changes, cursor: cursor)
        }
        func saveAll(_ mutations: [PendingMutation]) async throws { try await inner.saveAll(mutations) }
        func loadAll() async throws -> [PendingMutation] { try await inner.loadAll() }
    }

    private func tempPath() -> String {
        NSTemporaryDirectory() + "pylon_live_batch_\(UUID().uuidString).db"
    }

    private func makeEngine(_ persistence: SyncPersistence) async -> SyncEngine {
        let transport = MockTransport { req in
            switch req.url?.path ?? "" {
            case "/api/auth/me":
                return (200, Data(#"{"user_id":"u1","tenant_id":null,"is_admin":false,"roles":[]}"#.utf8))
            case "/api/sync/pull":
                return (200, Data(#"{"changes":[],"cursor":{"last_seq":0},"has_more":false}"#.utf8))
            default:
                return (404, Data("{}".utf8))
            }
        }
        let client = PylonClient(
            config: PylonClientConfig(baseURL: URL(string: "http://test.invalid")!),
            storage: MemoryStorage(),
            transport: transport
        )
        let cfg = SyncEngineConfig(baseURL: URL(string: "http://test.invalid")!, transport: .poll, pollInterval: 60)
        let engine = await SyncEngine(config: cfg, client: client, persistence: persistence)
        await engine.refreshResolvedSession()
        await engine.pull()
        return engine
    }

    private func frame(_ seq: Int, _ id: String, kind: String = "insert", _ fields: String = "") -> String {
        let data = kind == "delete" ? "" : #","data":{"id":"\#(id)"\#(fields)}"#
        return #"{"seq":\#(seq),"entity":"Msg","row_id":"\#(id)","kind":"\#(kind)"\#(data)}"#
    }

    func testABurstOfFramesSharesTransactions() async throws {
        let path = tempPath()
        defer { try? FileManager.default.removeItem(atPath: path) }
        let persistence = try CountingPersistence(path: path)
        let engine = await makeEngine(persistence)
        let n = 2000
        let start = Date()
        for i in 1...n {
            await engine.handleTextFrame(frame(i, "m\(i)", #","body":"hello \#(i)""#))
        }
        await engine.waitForPersistIdle()
        let batched = Date().timeIntervalSince(start)

        let rows = try await persistence.inner.loadAllRows()
        XCTAssertEqual(rows["Msg"]?.count, n)
        let cursor = try await persistence.inner.loadCursor()
        XCTAssertEqual(cursor?.last_seq, Int64(n))
        XCTAssertEqual(persistence.rowWrites, 0, "no per-row writes on the live path")
        XCTAssertLessThan(persistence.batches, n / 4, "frames share transactions")

        // The per-frame pattern this replaced: row write, then cursor write.
        let basePath = tempPath()
        defer { try? FileManager.default.removeItem(atPath: basePath) }
        let base = try SQLitePersistence(path: basePath)
        let baseStart = Date()
        for i in 1...n {
            try await base.persist(ChangeEvent(seq: Int64(i), entity: "Msg", row_id: "m\(i)", kind: .insert,
                                               data: ["id": .string("m\(i)"), "body": .string("hello \(i)")]))
            try await base.saveCursor(SyncCursor(last_seq: Int64(i)))
        }
        let perFrame = Date().timeIntervalSince(baseStart)
        print(String(format: "[bench] %d frames: group commit %.0f ms (%d transactions, %.0f frames/s); per-frame writes %.0f ms (%d transactions, %.0f frames/s)",
                     n, batched * 1000, persistence.batches, Double(n) / batched,
                     perFrame * 1000, n * 2, Double(n) / perFrame))
    }

    func testOrderInsideABatchHoldsOnDisk() async throws {
        let path = tempPath()
        defer { try? FileManager.default.removeItem(atPath: path) }
        let persistence = try CountingPersistence(path: path)
        let engine = await makeEngine(persistence)
        await engine.handleTextFrame(frame(1, "a", #","v":1"#))
        await engine.handleTextFrame(frame(2, "a", kind: "update", #","v":2"#))
        await engine.handleTextFrame(frame(3, "b", #","v":1"#))
        await engine.handleTextFrame(frame(4, "b", kind: "delete"))
        await engine.handleTextFrame(frame(5, "c", #","v":1"#))
        await engine.handleTextFrame(frame(6, "c", kind: "update", #","v":3"#))
        await engine.waitForPersistIdle()
        let rows = try await persistence.inner.loadAllRows()
        let byId = Dictionary(uniqueKeysWithValues: (rows["Msg"] ?? []).map { ($0["id"]!.stringValue!, $0) })
        XCTAssertEqual(byId["a"]?["v"], .int(2))
        XCTAssertNil(byId["b"])
        XCTAssertEqual(byId["c"]?["v"], .int(3))
        let cursor = try await persistence.inner.loadCursor()
        XCTAssertEqual(cursor?.last_seq, 6)
    }

    func testAFailedBatchFreezesTheOnDiskCursor() async throws {
        let path = tempPath()
        defer { try? FileManager.default.removeItem(atPath: path) }
        let persistence = try CountingPersistence(path: path)
        let engine = await makeEngine(persistence)
        await engine.handleTextFrame(frame(1, "a"))
        await engine.waitForPersistIdle()
        persistence.failNextBatch = true
        await engine.handleTextFrame(frame(2, "b"))
        await engine.waitForPersistIdle()
        await engine.handleTextFrame(frame(3, "c"))
        await engine.waitForPersistIdle()
        let memory = await engine.currentCursor().last_seq
        XCTAssertEqual(memory, 3, "memory stays authoritative")
        let cursor = try await persistence.inner.loadCursor()
        XCTAssertEqual(cursor?.last_seq, 1, "the on-disk cursor never passes a row that failed to write")
    }
}
