import XCTest

@testable import PylonRealtime

/// A transfer's ticket that expired before the client got through.
final class ShardTicketTests: XCTestCase {
    struct Nothing: Codable, Sendable {}

    /// A ticket shaped like the server's, expiring at `exp` (Unix seconds).
    static func ticket(exp: Double) -> String {
        let json = try! JSONSerialization.data(withJSONObject: ["shard": "east", "sid": "p1", "exp": exp])
        let payload = json.base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
        return "v1.\(payload).sig"
    }

    /// A transfer frame (kind 4, JSON) to `shard` with `ticket`.
    static func transfer(to shard: String, ticket: String) -> Data {
        var d = Data([4, 0])
        d.append(contentsOf: [UInt8](repeating: 0, count: 16))
        d.append(try! JSONSerialization.data(withJSONObject: ["shard": shard, "ticket": ticket]))
        return d
    }

    func testTicketExpiredReadsExp() {
        let now = Date(timeIntervalSince1970: 1_000_000_000)
        XCTAssertFalse(ShardClient<Nothing, Nothing>.ticketExpired(Self.ticket(exp: 1_000_000_600), now: now))
        XCTAssertTrue(ShardClient<Nothing, Nothing>.ticketExpired(Self.ticket(exp: 999_999_999), now: now))
        XCTAssertTrue(ShardClient<Nothing, Nothing>.ticketExpired(Self.ticket(exp: 1_000_000_002), now: now))
        XCTAssertFalse(ShardClient<Nothing, Nothing>.ticketExpired("not-a-ticket", now: now))
    }

    func testAnExpiredTransferTicketGivesWayToTheProvider() async {
        let client = ShardClient<Nothing, Nothing>(
            shardId: "west",
            config: ShardClientConfig(
                baseURL: URL(string: "http://h")!, subscriberId: "p1",
                ticketProvider: { shard in "own-\(shard)" }))
        let stale = Self.ticket(exp: Date().timeIntervalSince1970 - 60)
        await client.handleFrame(Self.transfer(to: "east", ticket: stale))
        let next = await client.nextTicket()
        XCTAssertEqual(next, "own-east")
    }

    func testAnExpiredTransferTicketForAnotherShardWithAFixedTicketStops() async {
        let client = ShardClient<Nothing, Nothing>(
            shardId: "west",
            config: ShardClientConfig(baseURL: URL(string: "http://h")!, subscriberId: "p1", ticket: "for-west"))
        let stale = Self.ticket(exp: Date().timeIntervalSince1970 - 60)
        await client.handleFrame(Self.transfer(to: "east", ticket: stale))
        let next = await client.nextTicket()
        XCTAssertNil(next)
        let left = await client.transferTicket
        XCTAssertNil(left)
    }

    func testTheSameShardOnAnotherMachineUsesTheFixedTicketOnceItsOwnExpires() async {
        let client = ShardClient<Nothing, Nothing>(
            shardId: "west",
            config: ShardClientConfig(baseURL: URL(string: "http://h")!, subscriberId: "p1", ticket: "for-west"))
        let fresh = Self.ticket(exp: Date().timeIntervalSince1970 + 600)
        await client.handleFrame(Self.transfer(to: "west", ticket: fresh))
        var next = await client.nextTicket()
        XCTAssertEqual(next, fresh)
        let stale = Self.ticket(exp: Date().timeIntervalSince1970 - 60)
        await client.handleFrame(Self.transfer(to: "west", ticket: stale))
        next = await client.nextTicket()
        XCTAssertEqual(next, "for-west")
    }
}
