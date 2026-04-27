import Foundation

/// SSR / hydration helper. Mirrors `getServerData` from
/// `packages/sync/src/index.ts`.
///
/// On the server (vapor / hummingbird / a Cloud Run handler), call this
/// once per request, ship the resulting `HydrationData` to the client,
/// then `engine.hydrate(data)` to skip the initial pull.
///
/// Failures on individual entity fetches degrade silently to an empty
/// list — callers don't need to handle each entity, since the next pull
/// will fill in any gaps.
public extension PylonClient {
    /// Fetch a snapshot of `entities` plus the current sync cursor.
    func getServerData(entities: [String]) async -> HydrationData {
        var entityData: [String: [[String: JSONValue]]] = [:]
        for entity in entities {
            do {
                let rows: [[String: JSONValue]] = try await list(entity)
                entityData[entity] = rows
            } catch {
                entityData[entity] = []
            }
        }
        var cursor = SyncCursor(last_seq: 0)
        if let pull = try? await syncPull(since: 0) {
            cursor = pull.cursor
        }
        return HydrationData(entities: entityData, cursor: cursor)
    }
}
