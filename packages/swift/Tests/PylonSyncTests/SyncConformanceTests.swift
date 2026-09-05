import XCTest
import PylonClient
@testable import PylonSync

/// Shared sync-engine conformance scenarios.
///
/// Runs every JSON file under `packages/sync/conformance/` against the
/// Swift engine. The TS engine runs the same files
/// (`packages/sync/src/conformance.test.ts`), so a behavior fixed in one
/// engine is pinned for the other. See `conformance/README.md`.
final class SyncConformanceTests: XCTestCase {

    struct Scenario: Decodable {
        let name: String
        let steps: [Step]
    }

    struct Step: Decodable {
        let op: String
        let entity: String?
        let row_id: String?
        let id: String?
        let kind: ChangeKind?
        let data: [String: JSONValue]?
        let frame: JSONValue?
        let present: Bool?
        let fields: [String: JSONValue]?
        let count: Int?
        let last_seq: Int64?
    }

    static var conformanceDir: URL {
        // Tests/PylonSyncTests/<this file> → packages/sync/conformance
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent() // PylonSyncTests
            .deletingLastPathComponent() // Tests
            .deletingLastPathComponent() // swift
            .deletingLastPathComponent() // packages
            .appendingPathComponent("sync/conformance", isDirectory: true)
    }

    static func scenarioFiles() throws -> [URL] {
        let files = try FileManager.default.contentsOfDirectory(
            at: conformanceDir, includingPropertiesForKeys: nil)
        return files.filter { $0.pathExtension == "json" }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }
    }

    func testEveryScenarioFileIsPresent() throws {
        let files = try Self.scenarioFiles()
        XCTAssertGreaterThanOrEqual(files.count, 6, "conformance dir: \(Self.conformanceDir.path)")
    }

    func testAllScenarios() async throws {
        for file in try Self.scenarioFiles() {
            let scenario = try JSONDecoder().decode(Scenario.self, from: Data(contentsOf: file))
            try await run(scenario, file: file.lastPathComponent)
        }
    }

    private func run(_ scenario: Scenario, file: String) async throws {
        let server = SyncEngineParityP1Tests.Server()
        let transport = MockTransport()
        transport.setHandler { [server] req in try await server.handle(req) }
        let client = PylonClient(
            config: PylonClientConfig(baseURL: URL(string: "http://test.invalid")!),
            storage: MemoryStorage(),
            transport: transport
        )
        await client.setSession(token: "tok1")
        let cfg = SyncEngineConfig(
            baseURL: URL(string: "http://test.invalid")!, transport: .poll, pollInterval: 60)
        let engine = await SyncEngine(config: cfg, client: client)
        await engine.refreshResolvedSession()
        let store = await engine.store

        for (index, step) in scenario.steps.enumerated() {
            let at = "\(file) step \(index + 1) (\(step.op))"
            switch step.op {
            case "seed":
                let seq = await server.seq + 1
                await server.seed(ChangeEvent(
                    seq: seq, entity: step.entity!, row_id: step.row_id!, kind: step.kind!,
                    data: step.data, timestamp: ""))
            case "pull":
                await engine.pull()
            case "frame":
                let bytes = try JSONEncoder().encode(step.frame!)
                await engine.handleTextFrame(String(decoding: bytes, as: UTF8.self))
            case "update":
                await engine.update(step.entity!, id: step.id!, step.data ?? [:])
            case "delete":
                await engine.delete(step.entity!, id: step.id!)
            case "expectRow":
                let row = store.get(step.entity!, id: step.id!)
                if step.present == false {
                    XCTAssertNil(row, at)
                    continue
                }
                XCTAssertNotNil(row, at)
                for (key, value) in step.fields ?? [:] {
                    XCTAssertEqual(row?[key], value, "\(at) field \(key)")
                }
            case "expectCount":
                XCTAssertEqual(store.list(step.entity!).count, step.count!, at)
            case "expectCursor":
                let cursor = await engine.currentCursor()
                XCTAssertEqual(cursor.last_seq, step.last_seq!, at)
            default:
                XCTFail("\(at): unknown op")
            }
        }
    }
}
