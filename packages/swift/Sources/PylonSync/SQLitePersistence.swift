import Foundation
import CSQLite
import PylonClient

/// SQLite-backed implementation of `SyncPersistence`. Stores entity rows,
/// the sync cursor, and the offline mutation queue in a single file.
/// Mirrors the IndexedDB schema (`packages/sync/src/persistence.ts`)
/// translated to SQL.
///
/// Schema (created on first open):
///   rows(entity TEXT, row_id TEXT, data TEXT, PRIMARY KEY(entity, row_id))
///   cursors(key TEXT PRIMARY KEY, last_seq INTEGER)
///   mutations(id TEXT PRIMARY KEY, payload TEXT)
///
/// All access funnels through a serial dispatch queue so calls from any
/// async context are safe without holding a sync lock across `await`.
public final class SQLitePersistence: SyncPersistence, @unchecked Sendable {
    private var db: OpaquePointer?
    private let queue = DispatchQueue(label: "pylon.sync.sqlite")
    private let path: String
    private let encoder: JSONEncoder
    private let decoder: JSONDecoder

    public init(path: String) throws {
        self.path = path
        self.encoder = JSONEncoder()
        self.decoder = JSONDecoder()
        try open()
    }

    deinit {
        if let db = db { sqlite3_close(db) }
    }

    private func open() throws {
        var handle: OpaquePointer?
        let flags = SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_FULLMUTEX
        if sqlite3_open_v2(path, &handle, flags, nil) != SQLITE_OK {
            let msg = handle.flatMap { String(cString: sqlite3_errmsg($0)) } ?? "unknown"
            if let h = handle { sqlite3_close(h) }
            throw PylonError.io(NSError(domain: "PylonSync.SQLite", code: 1, userInfo: [NSLocalizedDescriptionKey: "sqlite3_open: \(msg)"]))
        }
        self.db = handle
        try exec("PRAGMA journal_mode=WAL;")
        try exec("PRAGMA synchronous=NORMAL;")
        try exec("""
            CREATE TABLE IF NOT EXISTS rows (
                entity TEXT NOT NULL,
                row_id TEXT NOT NULL,
                data TEXT NOT NULL,
                PRIMARY KEY (entity, row_id)
            );
        """)
        try exec("""
            CREATE TABLE IF NOT EXISTS cursors (
                key TEXT PRIMARY KEY,
                last_seq INTEGER NOT NULL
            );
        """)
        try exec("""
            CREATE TABLE IF NOT EXISTS mutations (
                id TEXT PRIMARY KEY,
                payload TEXT NOT NULL
            );
        """)
    }

    private func exec(_ sql: String) throws {
        var err: UnsafeMutablePointer<CChar>?
        if sqlite3_exec(db, sql, nil, nil, &err) != SQLITE_OK {
            let msg = err.map { String(cString: $0) } ?? "unknown"
            sqlite3_free(err)
            throw PylonError.io(NSError(domain: "PylonSync.SQLite", code: 2, userInfo: [NSLocalizedDescriptionKey: msg]))
        }
    }

    // MARK: - SyncPersistence

    public func loadAllRows() async throws -> [String: [Row]] {
        try await withQueue {
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(self.db, "SELECT entity, data FROM rows", -1, &stmt, nil) == SQLITE_OK else {
                throw self.lastError()
            }
            defer { sqlite3_finalize(stmt) }

            var result: [String: [Row]] = [:]
            while sqlite3_step(stmt) == SQLITE_ROW {
                guard let entityCStr = sqlite3_column_text(stmt, 0),
                      let dataCStr = sqlite3_column_text(stmt, 1) else { continue }
                let entity = String(cString: entityCStr)
                let raw = Data(String(cString: dataCStr).utf8)
                if let row = try? self.decoder.decode(Row.self, from: raw) {
                    result[entity, default: []].append(row)
                }
            }
            return result
        }
    }

    public func loadCursor() async throws -> SyncCursor? {
        try await withQueue {
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(self.db, "SELECT last_seq FROM cursors WHERE key = 'cursor'", -1, &stmt, nil) == SQLITE_OK else {
                throw self.lastError()
            }
            defer { sqlite3_finalize(stmt) }
            if sqlite3_step(stmt) == SQLITE_ROW {
                return SyncCursor(last_seq: sqlite3_column_int64(stmt, 0))
            }
            return nil
        }
    }

    public func saveCursor(_ cursor: SyncCursor) async throws {
        try await withQueue {
            try self.execStatement(
                "INSERT OR REPLACE INTO cursors (key, last_seq) VALUES (?, ?)"
            ) { stmt in
                sqlite3_bind_text(stmt, 1, "cursor", -1, Self.SQLITE_TRANSIENT)
                sqlite3_bind_int64(stmt, 2, cursor.last_seq)
            }
        }
    }

    public func persist(_ change: ChangeEvent) async throws {
        try await withQueue {
            switch change.kind {
            case .insert, .update:
                guard let data = change.data else { return }
                let json = try self.encoder.encode(data)
                guard let s = String(data: json, encoding: .utf8) else { return }
                try self.execStatement(
                    "INSERT OR REPLACE INTO rows (entity, row_id, data) VALUES (?, ?, ?)"
                ) { stmt in
                    sqlite3_bind_text(stmt, 1, change.entity, -1, Self.SQLITE_TRANSIENT)
                    sqlite3_bind_text(stmt, 2, change.row_id, -1, Self.SQLITE_TRANSIENT)
                    sqlite3_bind_text(stmt, 3, s, -1, Self.SQLITE_TRANSIENT)
                }
            case .delete:
                try self.execStatement(
                    "DELETE FROM rows WHERE entity = ? AND row_id = ?"
                ) { stmt in
                    sqlite3_bind_text(stmt, 1, change.entity, -1, Self.SQLITE_TRANSIENT)
                    sqlite3_bind_text(stmt, 2, change.row_id, -1, Self.SQLITE_TRANSIENT)
                }
            }
        }
    }

    // MARK: - MutationQueuePersistence

    public func saveAll(_ mutations: [PendingMutation]) async throws {
        try await withQueue {
            try self.exec("BEGIN")
            do {
                try self.exec("DELETE FROM mutations")
                for m in mutations {
                    let payload = try self.encoder.encode(m)
                    guard let s = String(data: payload, encoding: .utf8) else { continue }
                    try self.execStatement(
                        "INSERT INTO mutations (id, payload) VALUES (?, ?)"
                    ) { stmt in
                        sqlite3_bind_text(stmt, 1, m.id, -1, Self.SQLITE_TRANSIENT)
                        sqlite3_bind_text(stmt, 2, s, -1, Self.SQLITE_TRANSIENT)
                    }
                }
                try self.exec("COMMIT")
            } catch {
                try? self.exec("ROLLBACK")
                throw error
            }
        }
    }

    public func loadAll() async throws -> [PendingMutation] {
        try await withQueue {
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(self.db, "SELECT payload FROM mutations", -1, &stmt, nil) == SQLITE_OK else {
                throw self.lastError()
            }
            defer { sqlite3_finalize(stmt) }
            var result: [PendingMutation] = []
            while sqlite3_step(stmt) == SQLITE_ROW {
                guard let cStr = sqlite3_column_text(stmt, 0) else { continue }
                let raw = Data(String(cString: cStr).utf8)
                if let m = try? self.decoder.decode(PendingMutation.self, from: raw) {
                    result.append(m)
                }
            }
            return result
        }
    }

    // MARK: - Helpers

    private func execStatement(_ sql: String, bind: (OpaquePointer?) -> Void) throws {
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else {
            throw lastError()
        }
        defer { sqlite3_finalize(stmt) }
        bind(stmt)
        let result = sqlite3_step(stmt)
        guard result == SQLITE_DONE || result == SQLITE_ROW else {
            throw lastError()
        }
    }

    private func lastError() -> PylonError {
        let msg = String(cString: sqlite3_errmsg(db))
        return .io(NSError(domain: "PylonSync.SQLite", code: 3, userInfo: [NSLocalizedDescriptionKey: msg]))
    }

    /// Sentinel passed to `sqlite3_bind_text` instructing SQLite to copy
    /// the buffer immediately. Safer than `SQLITE_STATIC` for short-lived
    /// Swift `String` storage.
    private static let SQLITE_TRANSIENT = unsafeBitCast(
        OpaquePointer(bitPattern: -1),
        to: sqlite3_destructor_type.self
    )

    private func withQueue<T: Sendable>(_ body: @escaping () throws -> T) async throws -> T {
        try await withCheckedThrowingContinuation { (cont: CheckedContinuation<T, Error>) in
            queue.async {
                do {
                    cont.resume(returning: try body())
                } catch {
                    cont.resume(throwing: error)
                }
            }
        }
    }
}
