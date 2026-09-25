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

/// A transfer frame moves the client to the new shard with the ticket it
/// carries, once; later connections ask the provider for the new shard.
final class ShardTransferTests: XCTestCase {
    struct Nothing: Codable, Sendable {}

    actor Asked {
        var shards: [String] = []
        func add(_ s: String) { shards.append(s) }
    }

    func testATransferFrameSwitchesTheShardAndUsesItsTicketOnce() async throws {
        let asked = Asked()
        let client = ShardClient<Nothing, Nothing>(
            shardId: "west",
            config: ShardClientConfig(
                baseURL: URL(string: "http://h")!,
                subscriberId: "p1",
                ticketProvider: { shard in
                    await asked.add(shard)
                    return "own-\(shard)"
                }))
        var url = await client.deriveURL()
        XCTAssertTrue(url.absoluteString.contains("shard=west"), url.absoluteString)

        let notice = Array(#"{"shard":"east","ticket":"from-server"}"#.utf8)
        await client.handleFrame(ShardBackoffTests.frame(kind: 4, codec: 0, payload: notice))
        let shard = await client.shardId
        XCTAssertEqual(shard, "east")
        url = await client.deriveURL()
        XCTAssertTrue(url.absoluteString.contains("shard=east"), url.absoluteString)
        // Until the new shard answers, every attempt uses the server's ticket.
        var ticket = await client.nextTicket()
        XCTAssertEqual(ticket, "from-server")
        ticket = await client.nextTicket()
        XCTAssertEqual(ticket, "from-server")
        await client.handleFrame(ShardBackoffTests.frame(kind: 1, codec: 0, payload: Array("{}".utf8)))
        ticket = await client.nextTicket()
        XCTAssertEqual(ticket, "own-east")
        let shards = await asked.shards
        XCTAssertEqual(shards, ["east"])
    }

    func testAnExplicitURLFollowsATransfer() async {
        let client = ShardClient<Nothing, Nothing>(
            shardId: "west",
            config: ShardClientConfig(
                baseURL: URL(string: "http://h")!,
                subscriberId: "p1",
                wsURL: URL(string: "ws://h/shard?shard=west&sid=p1&v=2")!))
        let notice = Array(#"{"shard":"east","ticket":"t"}"#.utf8)
        await client.handleFrame(ShardBackoffTests.frame(kind: 4, codec: 0, payload: notice))
        let url = await client.deriveURL()
        XCTAssertEqual(url.absoluteString, "ws://h/shard?shard=east&sid=p1&v=2")
    }
}

final class ShardTransferFixedTicketTests: XCTestCase {
    struct Nothing: Codable, Sendable {}

    /// A fixed ticket names the first shard: after a transfer it is never
    /// sent again.
    func testAFixedTicketIsNotSentAfterATransfer() async {
        let client = ShardClient<Nothing, Nothing>(
            shardId: "west",
            config: ShardClientConfig(
                baseURL: URL(string: "http://h")!, subscriberId: "p1", ticket: "for-west"))
        var ticket = await client.nextTicket()
        XCTAssertEqual(ticket, "for-west")
        let notice = Array(#"{"shard":"east","ticket":"for-east"}"#.utf8)
        await client.handleFrame(ShardBackoffTests.frame(kind: 4, codec: 0, payload: notice))
        await client.handleFrame(ShardBackoffTests.frame(kind: 1, codec: 0, payload: Array("{}".utf8)))
        ticket = await client.nextTicket()
        XCTAssertEqual(ticket, "for-east")
    }
}
