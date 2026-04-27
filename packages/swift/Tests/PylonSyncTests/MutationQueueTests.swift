import XCTest
import PylonClient
@testable import PylonSync

final class MutationQueueTests: XCTestCase {

    func testAddSetsOpIdOnOutgoingChange() async {
        let queue = MutationQueue()
        let id = await queue.add(ClientChange(entity: "Todo", row_id: "t1", kind: .insert, data: ["title": "x"]))
        let pending = await queue.pending()
        XCTAssertEqual(pending.count, 1)
        XCTAssertEqual(pending[0].id, id)
        XCTAssertEqual(pending[0].change.op_id, id, "op_id on the wire must equal the queue id so server can dedupe replays")
    }

    func testClearKeepsFailedDropsApplied() async {
        let queue = MutationQueue()
        let id1 = await queue.add(ClientChange(entity: "Todo", row_id: "t1", kind: .insert))
        let id2 = await queue.add(ClientChange(entity: "Todo", row_id: "t2", kind: .insert))
        await queue.markApplied(id1)
        await queue.markFailed(id2, error: "validation failed")
        await queue.clear()
        let all = await queue.all()
        XCTAssertEqual(all.count, 1)
        XCTAssertEqual(all[0].id, id2)
        XCTAssertEqual(all[0].status, .failed, "failed mutations must stay so the UI can ack/retry")
    }

    func testHydrateMergesPersistedState() async throws {
        actor MemoryPersistence: MutationQueuePersistence {
            var stored: [PendingMutation] = []
            func saveAll(_ mutations: [PendingMutation]) async throws { stored = mutations }
            func loadAll() async throws -> [PendingMutation] { stored }
        }
        let mem = MemoryPersistence()
        // Seed with an existing mutation.
        try await mem.saveAll([
            PendingMutation(
                id: "mut_seeded",
                change: ClientChange(entity: "Todo", row_id: "t1", kind: .insert, op_id: "mut_seeded"),
                status: .pending
            )
        ])
        let queue = MutationQueue(persistence: mem)
        await queue.hydrate()
        let pending = await queue.pending()
        XCTAssertEqual(pending.count, 1)
        XCTAssertEqual(pending[0].id, "mut_seeded")
    }
}

final class BackoffTests: XCTestCase {
    func testBackoffStaysWithinBounds() {
        for attempt in 1...10 {
            let delay = computeBackoff(attempts: attempt, baseDelay: 1.0, maxDelay: 30.0)
            XCTAssertGreaterThanOrEqual(delay, 0)
            XCTAssertLessThanOrEqual(delay, 30.0)
        }
    }

    func testBackoffGrowsBeforeCap() {
        // Seed-free property: averaged over many trials, attempt N should
        // not exceed attempt N+5's mean. Use enough trials to wash out
        // jitter.
        var sum1: Double = 0
        var sum6: Double = 0
        for _ in 0..<200 {
            sum1 += computeBackoff(attempts: 1, baseDelay: 1.0)
            sum6 += computeBackoff(attempts: 6, baseDelay: 1.0)
        }
        XCTAssertLessThan(sum1 / 200, sum6 / 200, "deeper attempts should average higher delays")
    }
}
