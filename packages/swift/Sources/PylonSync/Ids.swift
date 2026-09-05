import Foundation

/// Pylon-shaped id generation. Mirrors `packages/sync/src/ids.ts`.
///
/// Ids are lex-sortable: a 32-char hex nanosecond timestamp followed by an
/// 8-char hex counter. Cursor pagination (`WHERE id > '<after>'`) relies
/// on that shape, and a 39-char id would sort ahead of a 40-char one.
public enum PylonIds {
    private static let lock = NSLock()
    private static var counter: UInt32 = 0

    /// Mint a 40-hex-char id. `SyncEngine.insert` uses it so the optimistic
    /// ghost and the canonical server row share one id.
    public static func generate() -> String {
        let nanos = UInt64(Date().timeIntervalSince1970 * 1_000_000_000)
        lock.lock()
        let seq = counter
        counter &+= 1
        lock.unlock()
        let head = String(nanos, radix: 16)
        let tail = String(seq, radix: 16)
        return String(repeating: "0", count: max(0, 32 - head.count)) + head
            + String(repeating: "0", count: max(0, 8 - tail.count)) + tail
    }
}
