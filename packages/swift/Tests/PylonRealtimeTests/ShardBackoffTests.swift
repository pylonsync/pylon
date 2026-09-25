import XCTest
@testable import PylonRealtime

/// The reconnect backoff resets only when a frame applies. A frame that
/// always fails must not bring the delay back to the shortest one.
final class ShardBackoffTests: XCTestCase {
    struct Nothing: Codable, Sendable {}

    static func frame(kind: UInt8, codec: UInt8, payload: [UInt8]) -> Data {
        var d = Data([kind, codec])
        d.append(contentsOf: [UInt8](repeating: 0, count: 16))  // tick, ack
        d.append(contentsOf: payload)
        return d
    }

    /// An empty full frame: version 1, FULL, precision 0.01, no entities.
    static let goodReplication = frame(
        kind: 3, codec: 4, payload: [1, 1, 0x0A, 0xD7, 0x23, 0x3C, 0, 0, 0])
    /// Version 2: this client can never apply it.
    static let badReplication = frame(
        kind: 3, codec: 4, payload: [2, 1, 0x0A, 0xD7, 0x23, 0x3C, 0, 0, 0])

    func makeClient() -> ShardClient<Nothing, Nothing> {
        ShardClient(
            shardId: "zone",
            config: ShardClientConfig(baseURL: URL(string: "http://h")!, subscriberId: "p1"))
    }

    func testAFailedFrameKeepsTheBackoffAndAnAppliedFrameResetsIt() async {
        let client = makeClient()
        for _ in 0..<3 { await client.noteReconnect() }
        await client.handleFrame(Self.badReplication)
        var attempts = await client.reconnectAttempts
        XCTAssertEqual(attempts, 3)
        await client.handleFrame(Self.goodReplication)
        attempts = await client.reconnectAttempts
        XCTAssertEqual(attempts, 0)
    }

    func testADecodedSnapshotResetsTheBackoff() async {
        let client = makeClient()
        await client.noteReconnect()
        await client.handleFrame(Self.frame(kind: 1, codec: 0, payload: Array("not json".utf8)))
        var attempts = await client.reconnectAttempts
        XCTAssertEqual(attempts, 1)
        await client.handleFrame(Self.frame(kind: 1, codec: 0, payload: Array("{}".utf8)))
        attempts = await client.reconnectAttempts
        XCTAssertEqual(attempts, 0)
    }
}
