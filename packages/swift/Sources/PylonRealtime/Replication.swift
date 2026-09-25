import Foundation

/// Entity replication frames (`pylon_replication::frame` in Rust). A shard
/// that replicates entities sends each subscriber, per tick, despawns for
/// entities that left its view, spawns (full state) for entities that came
/// into view, and updates carrying only the axes that moved (as quantized
/// differences) and the components that changed or were removed.
/// `EntityTable` applies them.
public enum ReplicationError: Error, Equatable, CustomStringConvertible {
    case malformed(String)

    public var description: String {
        switch self {
        case .malformed(let why): return "replication frame: \(why)"
        }
    }
}

/// One entity as the client sees it.
public struct ReplicatedEntity: Sendable, Equatable {
    public let id: UInt64
    /// Quantized position; `position` is this times the frame precision.
    public var q: [Int64]
    public var position: (x: Double, y: Double, z: Double)
    /// Component id to bytes, in the game's own encoding.
    public var components: [UInt8: Data]

    public static func == (a: ReplicatedEntity, b: ReplicatedEntity) -> Bool {
        a.id == b.id && a.q == b.q && a.components == b.components
    }
}

/// What one frame did.
public struct ReplicationSummary: Sendable, Equatable {
    public var full = false
    public var spawned: [UInt64] = []
    public var updated: [UInt64] = []
    public var despawned: [UInt64] = []
}

/// The entities one subscriber has been told about. Apply every frame in
/// order; a frame that fails to apply means the connection is out of sync
/// and should be reopened (the server then sends a full frame).
public struct EntityTable: Sendable {
    public private(set) var entities: [UInt64: ReplicatedEntity] = [:]
    /// World units per quantization step, from the last frame.
    public private(set) var precision: Float = 0.01

    public init() {}

    public var count: Int { entities.count }

    public subscript(id: UInt64) -> ReplicatedEntity? { entities[id] }

    public mutating func clear() { entities.removeAll() }

    private static let maxCount: UInt64 = 1 << 24

    @discardableResult
    public mutating func apply(_ frame: Data) throws -> ReplicationSummary {
        var r = Reader(bytes: [UInt8](frame))
        let version = try r.u8()
        guard version == 1 else { throw ReplicationError.malformed("version \(version)") }
        let full = (try r.u8() & 1) != 0
        let precision = try r.f32()
        guard precision.isFinite, precision > 0 else { throw ReplicationError.malformed("bad precision") }
        if full { entities.removeAll() }
        self.precision = precision
        var summary = ReplicationSummary(full: full)

        var n = try count(&r)
        var last: UInt64? = nil
        for _ in 0..<n {
            let id = try nextId(&r, &last)
            entities[id] = nil
            summary.despawned.append(id)
        }

        n = try count(&r)
        last = nil
        for _ in 0..<n {
            let id = try nextId(&r, &last)
            let q = [try r.zigzag(), try r.zigzag(), try r.zigzag()]
            var components: [UInt8: Data] = [:]
            try readComponents(&r, &components)
            entities[id] = ReplicatedEntity(id: id, q: q, position: (0, 0, 0), components: components)
            summary.spawned.append(id)
        }

        n = try count(&r)
        last = nil
        for _ in 0..<n {
            let id = try nextId(&r, &last)
            let mask = try r.u8()
            guard var e = entities[id] else {
                throw ReplicationError.malformed("update for unknown entity \(id)")
            }
            for (i, bit) in [UInt8(1), 2, 4].enumerated() where mask & bit != 0 {
                // Wrapping, like the encoder: exact over the whole Int64 range.
                e.q[i] = e.q[i] &+ (try r.zigzag())
            }
            if mask & 8 != 0 { try readComponents(&r, &e.components) }
            entities[id] = e
            summary.updated.append(id)
        }
        guard r.done else { throw ReplicationError.malformed("trailing bytes") }

        let p = Double(precision)
        for (id, var e) in entities {
            e.position = (Double(e.q[0]) * p, Double(e.q[1]) * p, Double(e.q[2]) * p)
            entities[id] = e
        }
        return summary
    }

    private func count(_ r: inout Reader) throws -> Int {
        let n = try r.varint()
        guard n <= Self.maxCount else { throw ReplicationError.malformed("count too large") }
        return Int(n)
    }

    private func nextId(_ r: inout Reader, _ last: inout UInt64?) throws -> UInt64 {
        let v = try r.varint()
        let id: UInt64
        if let prev = last {
            let (sum, overflow) = prev.addingReportingOverflow(v)
            guard v > 0, !overflow else { throw ReplicationError.malformed("ids do not ascend") }
            id = sum
        } else {
            id = v
        }
        last = id
        return id
    }

    private func readComponents(_ r: inout Reader, _ into: inout [UInt8: Data]) throws {
        let n = try r.varint()
        guard n <= 256 else { throw ReplicationError.malformed("more than 256 components") }
        for _ in 0..<Int(n) {
            let id = try r.u8()
            let len = try r.varint()
            if len == 0 {
                into[id] = nil
                continue
            }
            into[id] = try r.take(len - 1)
        }
    }
}

private struct Reader {
    let bytes: [UInt8]
    var offset = 0

    var done: Bool { offset >= bytes.count }

    mutating func u8() throws -> UInt8 {
        guard offset < bytes.count else { throw ReplicationError.malformed("ends early") }
        defer { offset += 1 }
        return bytes[offset]
    }

    mutating func f32() throws -> Float {
        guard offset + 4 <= bytes.count else { throw ReplicationError.malformed("ends early") }
        var bits: UInt32 = 0
        for i in 0..<4 { bits |= UInt32(bytes[offset + i]) << (8 * UInt32(i)) }
        offset += 4
        return Float(bitPattern: bits)
    }

    mutating func varint() throws -> UInt64 {
        var v: UInt64 = 0
        for i in 0..<10 {
            let b = try u8()
            let part = UInt64(b & 0x7f)
            if i == 9 && part > 1 { throw ReplicationError.malformed("varint past 64 bits") }
            v |= part << UInt64(7 * i)
            if b & 0x80 == 0 { return v }
        }
        throw ReplicationError.malformed("varint too long")
    }

    mutating func zigzag() throws -> Int64 {
        let z = try varint()
        return Int64(bitPattern: z >> 1) ^ -Int64(bitPattern: z & 1)
    }

    mutating func take(_ n: UInt64) throws -> Data {
        guard n <= UInt64(bytes.count - offset) else {
            throw ReplicationError.malformed("component runs past the end")
        }
        let end = offset + Int(n)
        defer { offset = end }
        return Data(bytes[offset..<end])
    }
}
