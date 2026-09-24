import XCTest
@testable import PylonRealtime

/// End-to-end: ShardClient against a real MessagePack shard. Runs only when
/// PYLON_SHARD_E2E holds the JSON line printed by
/// `cargo run -p pylon-runtime --example shard_codec_server`
/// (tools/smoke-shard-codecs.sh sets it up).
final class ShardE2ETests: XCTestCase {
    struct Arena: Decodable, Sendable {
        struct Player: Decodable, Sendable, Equatable {
            let id: String
            let x: Int
            let y: Int
        }
        let players: [Player]
        let label: String
    }

    struct Move: Encodable, Sendable {
        let dx: Int
        let dy: Int
    }

    struct Server: Decodable {
        let port: Int
        let shard: String
        let tokens: [String: String]
    }

    func testMessagePackSnapshotsBinaryInputsAcksAndRejections() async throws {
        guard let raw = ProcessInfo.processInfo.environment["PYLON_SHARD_E2E"] else {
            throw XCTSkip("PYLON_SHARD_E2E is not set")
        }
        let server = try JSONDecoder().decode(Server.self, from: Data(raw.utf8))
        let client = ShardClient<Arena, Move>(
            shardId: server.shard,
            config: ShardClientConfig(
                baseURL: URL(string: "http://127.0.0.1")!,
                subscriberId: "swift",
                token: server.tokens["swift"],
                wsPort: server.port,
                autoReconnect: false
            )
        )
        let snapshots = await client.snapshots()
        let rejections = await client.rejections()
        Task { await client.connect() }
        // A hang ends the streams instead of the test run: after 10 s the
        // client closes and every `next()` below returns nil.
        let watchdog = Task {
            try await Task.sleep(nanoseconds: 10_000_000_000)
            await client.close()
        }
        defer { watchdog.cancel() }

        var iterator = snapshots.makeAsyncIterator()
        let first = await iterator.next()
        XCTAssertEqual(first?.state.label, "arena")

        // The client learned the codec from the first frame: inputs now go as
        // binary MessagePack.
        for _ in 0..<3 { try await client.send(Move(dx: 1, dy: 2)) }
        var acked: ShardSnapshot<Arena>?
        while acked == nil, let snap = await iterator.next() {
            if snap.ack >= 3 { acked = snap }
        }
        XCTAssertEqual(
            acked?.state.players.first { $0.id == "swift" },
            Arena.Player(id: "swift", x: 3, y: 6)
        )

        let seq = try await client.send(Move(dx: -999, dy: 0))
        var rejectionIterator = rejections.makeAsyncIterator()
        let rejection = await rejectionIterator.next()
        XCTAssertEqual(rejection?.clientSeq, seq)
        XCTAssertEqual(rejection?.code, "apply_failed")

        await client.close()
    }
}
