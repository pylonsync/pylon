import Foundation

/// Synchronous key-value adapter for hot-path state (auth token, client_id).
///
/// Mirrors the TypeScript `Storage` interface in `packages/sync/src/storage.ts`.
/// Synchronous so the engine doesn't need to be async-all-the-way-down on every
/// `currentToken()` call. Hosts that only have async backends (Keychain on
/// iOS, AsyncStorage in RN) should hydrate the cache at startup and write
/// through in the background — see `MemoryStorage` and `WriteThroughStorage`.
public protocol PylonStorage: Sendable {
    func get(_ key: String) -> String?
    func set(_ key: String, value: String)
    func remove(_ key: String)
}

/// In-memory map. Default on Linux / non-Apple platforms. Tokens won't survive
/// process restarts.
public final class MemoryStorage: PylonStorage, @unchecked Sendable {
    private let lock = NSLock()
    private var map: [String: String] = [:]

    public init(_ seed: [String: String] = [:]) {
        self.map = seed
    }

    public func get(_ key: String) -> String? {
        lock.lock(); defer { lock.unlock() }
        return map[key]
    }

    public func set(_ key: String, value: String) {
        lock.lock(); defer { lock.unlock() }
        map[key] = value
    }

    public func remove(_ key: String) {
        lock.lock(); defer { lock.unlock() }
        map.removeValue(forKey: key)
    }
}

/// In-memory cache that mirrors writes to an async backend (Keychain,
/// AsyncStorage, KV). Reads are immediate. Writes are eventually-consistent.
///
/// Construct after seeding the cache from your async backend at startup —
/// otherwise the first read for a previously-saved key returns nil.
public final class WriteThroughStorage: PylonStorage, @unchecked Sendable {
    private let lock = NSLock()
    private var map: [String: String]
    private let onWrite: @Sendable (_ key: String, _ value: String?) -> Void

    public init(
        seed: [String: String] = [:],
        onWrite: @escaping @Sendable (_ key: String, _ value: String?) -> Void
    ) {
        self.map = seed
        self.onWrite = onWrite
    }

    public func get(_ key: String) -> String? {
        lock.lock(); defer { lock.unlock() }
        return map[key]
    }

    public func set(_ key: String, value: String) {
        lock.lock(); map[key] = value; lock.unlock()
        onWrite(key, value)
    }

    public func remove(_ key: String) {
        lock.lock(); map.removeValue(forKey: key); lock.unlock()
        onWrite(key, nil)
    }
}

/// `UserDefaults`-backed storage. Apple platforms only. The closest analog to
/// browser localStorage.
#if canImport(Darwin)
public final class UserDefaultsStorage: PylonStorage, @unchecked Sendable {
    private let defaults: UserDefaults
    private let prefix: String

    public init(suiteName: String? = nil, prefix: String = "pylon.") {
        self.defaults = suiteName.flatMap { UserDefaults(suiteName: $0) } ?? .standard
        self.prefix = prefix
    }

    private func k(_ key: String) -> String { prefix + key }

    public func get(_ key: String) -> String? {
        defaults.string(forKey: k(key))
    }

    public func set(_ key: String, value: String) {
        defaults.set(value, forKey: k(key))
    }

    public func remove(_ key: String) {
        defaults.removeObject(forKey: k(key))
    }
}
#endif

/// Pick a sensible default storage for the current platform: UserDefaults on
/// Apple platforms, in-memory elsewhere. Apps wanting Keychain-backed token
/// persistence should pass a custom `WriteThroughStorage`.
public func defaultPylonStorage() -> PylonStorage {
    #if canImport(Darwin)
    return UserDefaultsStorage()
    #else
    return MemoryStorage()
    #endif
}

// ---------------------------------------------------------------------------
// Storage key conventions
//
// The sync engine and the HTTP client agree on these key names so a token
// set by `PylonClient.setSession(...)` is read by `SyncEngine` automatically.
// ---------------------------------------------------------------------------

public enum StorageKeys {
    /// Stable per-client identifier. Persisted so reloads keep the same id.
    public static let clientId = "pylon:client_id"

    /// Auth token storage key, namespaced by appName. Matches the TS
    /// convention from `packages/react`'s `configureClient`.
    public static func token(appName: String = "default") -> String {
        appName == "default" ? "pylon_token" : "pylon:\(appName):token"
    }
}
