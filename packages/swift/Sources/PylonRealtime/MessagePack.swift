import Foundation

// MessagePack for the shard wire protocol: a value model, a reader and
// writer for the format, and a Codable decoder and encoder over it.
//
// Shards encode structs as maps with field names (rmp_serde's named form),
// so keyed Codable types decode directly. Extension types are not used by
// the protocol and are rejected.

/// One MessagePack value.
public indirect enum MessagePackValue: Equatable, Sendable {
    case nil_
    case bool(Bool)
    case int(Int64)
    case uint(UInt64)
    case float(Double)
    case string(String)
    case binary(Data)
    case array([MessagePackValue])
    /// Keys in the order they appear on the wire.
    case map([(MessagePackValue, MessagePackValue)])

    public static func == (lhs: MessagePackValue, rhs: MessagePackValue) -> Bool {
        switch (lhs, rhs) {
        case (.nil_, .nil_): return true
        case let (.bool(a), .bool(b)): return a == b
        case let (.int(a), .int(b)): return a == b
        case let (.uint(a), .uint(b)): return a == b
        case let (.int(a), .uint(b)), let (.uint(b), .int(a)): return a >= 0 && UInt64(a) == b
        case let (.float(a), .float(b)): return a == b
        case let (.string(a), .string(b)): return a == b
        case let (.binary(a), .binary(b)): return a == b
        case let (.array(a), .array(b)): return a == b
        case let (.map(a), .map(b)):
            return a.count == b.count && zip(a, b).allSatisfy { $0.0 == $1.0 && $0.1 == $1.1 }
        default: return false
        }
    }
}

public enum MessagePackError: Error, Equatable {
    case truncated
    case unsupportedType(UInt8)
    case invalidString
    case trailingBytes
}

// MARK: - Reading

public enum MessagePackReader {
    /// Parse exactly one value from `data`.
    public static func read(_ data: Data) throws -> MessagePackValue {
        var cursor = Cursor(bytes: [UInt8](data))
        let value = try cursor.value()
        guard cursor.offset == cursor.bytes.count else { throw MessagePackError.trailingBytes }
        return value
    }

    private struct Cursor {
        let bytes: [UInt8]
        var offset = 0

        mutating func byte() throws -> UInt8 {
            guard offset < bytes.count else { throw MessagePackError.truncated }
            defer { offset += 1 }
            return bytes[offset]
        }

        mutating func take(_ n: Int) throws -> ArraySlice<UInt8> {
            guard n >= 0, offset + n <= bytes.count else { throw MessagePackError.truncated }
            defer { offset += n }
            return bytes[offset..<(offset + n)]
        }

        mutating func uint(_ width: Int) throws -> UInt64 {
            var v: UInt64 = 0
            for b in try take(width) { v = (v << 8) | UInt64(b) }
            return v
        }

        mutating func string(_ n: Int) throws -> MessagePackValue {
            guard let s = String(bytes: try take(n), encoding: .utf8) else {
                throw MessagePackError.invalidString
            }
            return .string(s)
        }

        mutating func array(_ n: Int) throws -> MessagePackValue {
            var out: [MessagePackValue] = []
            out.reserveCapacity(min(n, 4096))
            for _ in 0..<n { out.append(try value()) }
            return .array(out)
        }

        mutating func map(_ n: Int) throws -> MessagePackValue {
            var out: [(MessagePackValue, MessagePackValue)] = []
            out.reserveCapacity(min(n, 4096))
            for _ in 0..<n { out.append((try value(), try value())) }
            return .map(out)
        }

        mutating func value() throws -> MessagePackValue {
            let b = try byte()
            switch b {
            case 0x00...0x7f: return .uint(UInt64(b))
            case 0x80...0x8f: return try map(Int(b & 0x0f))
            case 0x90...0x9f: return try array(Int(b & 0x0f))
            case 0xa0...0xbf: return try string(Int(b & 0x1f))
            case 0xc0: return .nil_
            case 0xc2: return .bool(false)
            case 0xc3: return .bool(true)
            case 0xc4: return .binary(Data(try take(Int(try uint(1)))))
            case 0xc5: return .binary(Data(try take(Int(try uint(2)))))
            case 0xc6: return .binary(Data(try take(Int(try uint(4)))))
            case 0xca: return .float(Double(Float(bitPattern: UInt32(try uint(4)))))
            case 0xcb: return .float(Double(bitPattern: try uint(8)))
            case 0xcc: return .uint(try uint(1))
            case 0xcd: return .uint(try uint(2))
            case 0xce: return .uint(try uint(4))
            case 0xcf: return .uint(try uint(8))
            case 0xd0: return .int(Int64(Int8(bitPattern: UInt8(try uint(1)))))
            case 0xd1: return .int(Int64(Int16(bitPattern: UInt16(try uint(2)))))
            case 0xd2: return .int(Int64(Int32(bitPattern: UInt32(try uint(4)))))
            case 0xd3: return .int(Int64(bitPattern: try uint(8)))
            case 0xd9: return try string(Int(try uint(1)))
            case 0xda: return try string(Int(try uint(2)))
            case 0xdb: return try string(Int(try uint(4)))
            case 0xdc: return try array(Int(try uint(2)))
            case 0xdd: return try array(Int(try uint(4)))
            case 0xde: return try map(Int(try uint(2)))
            case 0xdf: return try map(Int(try uint(4)))
            case 0xe0...0xff: return .int(Int64(Int8(bitPattern: b)))
            default: throw MessagePackError.unsupportedType(b)
            }
        }
    }
}

// MARK: - Writing

public enum MessagePackWriter {
    public static func write(_ value: MessagePackValue) -> Data {
        var out: [UInt8] = []
        append(value, to: &out)
        return Data(out)
    }

    private static func be(_ v: UInt64, _ width: Int, _ out: inout [UInt8]) {
        for i in stride(from: width - 1, through: 0, by: -1) {
            out.append(UInt8(truncatingIfNeeded: v >> (UInt64(i) * 8)))
        }
    }

    private static func append(_ value: MessagePackValue, to out: inout [UInt8]) {
        switch value {
        case .nil_: out.append(0xc0)
        case .bool(let b): out.append(b ? 0xc3 : 0xc2)
        case .uint(let v): appendUInt(v, &out)
        case .int(let v):
            if v >= 0 { appendUInt(UInt64(v), &out) }
            else if v >= -32 { out.append(UInt8(bitPattern: Int8(v))) }
            else if v >= Int64(Int8.min) { out.append(0xd0); be(UInt64(UInt8(bitPattern: Int8(v))), 1, &out) }
            else if v >= Int64(Int16.min) { out.append(0xd1); be(UInt64(UInt16(bitPattern: Int16(v))), 2, &out) }
            else if v >= Int64(Int32.min) { out.append(0xd2); be(UInt64(UInt32(bitPattern: Int32(v))), 4, &out) }
            else { out.append(0xd3); be(UInt64(bitPattern: v), 8, &out) }
        case .float(let d): out.append(0xcb); be(d.bitPattern, 8, &out)
        case .string(let s):
            let bytes = Array(s.utf8)
            if bytes.count < 32 { out.append(0xa0 | UInt8(bytes.count)) }
            else if bytes.count <= 0xff { out.append(0xd9); be(UInt64(bytes.count), 1, &out) }
            else if bytes.count <= 0xffff { out.append(0xda); be(UInt64(bytes.count), 2, &out) }
            else { out.append(0xdb); be(UInt64(bytes.count), 4, &out) }
            out.append(contentsOf: bytes)
        case .binary(let d):
            if d.count <= 0xff { out.append(0xc4); be(UInt64(d.count), 1, &out) }
            else if d.count <= 0xffff { out.append(0xc5); be(UInt64(d.count), 2, &out) }
            else { out.append(0xc6); be(UInt64(d.count), 4, &out) }
            out.append(contentsOf: d)
        case .array(let items):
            if items.count < 16 { out.append(0x90 | UInt8(items.count)) }
            else if items.count <= 0xffff { out.append(0xdc); be(UInt64(items.count), 2, &out) }
            else { out.append(0xdd); be(UInt64(items.count), 4, &out) }
            for item in items { append(item, to: &out) }
        case .map(let pairs):
            if pairs.count < 16 { out.append(0x80 | UInt8(pairs.count)) }
            else if pairs.count <= 0xffff { out.append(0xde); be(UInt64(pairs.count), 2, &out) }
            else { out.append(0xdf); be(UInt64(pairs.count), 4, &out) }
            for (k, v) in pairs {
                append(k, to: &out)
                append(v, to: &out)
            }
        }
    }

    private static func appendUInt(_ v: UInt64, _ out: inout [UInt8]) {
        if v < 0x80 { out.append(UInt8(v)) }
        else if v <= 0xff { out.append(0xcc); be(v, 1, &out) }
        else if v <= 0xffff { out.append(0xcd); be(v, 2, &out) }
        else if v <= 0xffff_ffff { out.append(0xce); be(v, 4, &out) }
        else { out.append(0xcf); be(v, 8, &out) }
    }
}

// MARK: - Decoder

/// Decodes a `Decodable` type from MessagePack bytes.
public struct MessagePackDecoder {
    public var userInfo: [CodingUserInfoKey: Any] = [:]

    public init() {}

    public func decode<T: Decodable>(_ type: T.Type, from data: Data) throws -> T {
        try decode(type, from: MessagePackReader.read(data))
    }

    public func decode<T: Decodable>(_ type: T.Type, from value: MessagePackValue) throws -> T {
        try _MPDecoder(value: value, codingPath: [], userInfo: userInfo).decodeValue(type)
    }
}

private struct _MPKey: CodingKey {
    var stringValue: String
    var intValue: Int?
    init(stringValue: String) { self.stringValue = stringValue }
    init(intValue: Int) {
        self.stringValue = String(intValue)
        self.intValue = intValue
    }
}

private struct _MPDecoder: Decoder {
    let value: MessagePackValue
    let codingPath: [CodingKey]
    let userInfo: [CodingUserInfoKey: Any]

    func decodeValue<T: Decodable>(_ type: T.Type) throws -> T {
        if type == Data.self {
            if case .binary(let d) = value { return d as! T }
            throw mismatch(type)
        }
        return try T(from: self)
    }

    func mismatch(_ type: Any.Type) -> DecodingError {
        DecodingError.typeMismatch(
            type,
            .init(codingPath: codingPath, debugDescription: "expected \(type), found \(value)")
        )
    }

    func container<Key: CodingKey>(keyedBy type: Key.Type) throws -> KeyedDecodingContainer<Key> {
        guard case .map(let pairs) = value else { throw mismatch([String: Any].self) }
        var dict: [String: MessagePackValue] = [:]
        for (k, v) in pairs {
            switch k {
            case .string(let s): dict[s] = v
            case .uint(let n): dict[String(n)] = v
            case .int(let n): dict[String(n)] = v
            default: continue
            }
        }
        return KeyedDecodingContainer(
            _MPKeyed<Key>(dict: dict, codingPath: codingPath, userInfo: userInfo))
    }

    func unkeyedContainer() throws -> UnkeyedDecodingContainer {
        guard case .array(let items) = value else { throw mismatch([Any].self) }
        return _MPUnkeyed(items: items, codingPath: codingPath, userInfo: userInfo)
    }

    func singleValueContainer() throws -> SingleValueDecodingContainer {
        _MPSingle(decoder: self)
    }
}

private struct _MPSingle: SingleValueDecodingContainer {
    let decoder: _MPDecoder
    var codingPath: [CodingKey] { decoder.codingPath }
    var value: MessagePackValue { decoder.value }

    func decodeNil() -> Bool {
        if case .nil_ = value { return true }
        return false
    }
    func decode(_ type: Bool.Type) throws -> Bool {
        if case .bool(let b) = value { return b }
        throw decoder.mismatch(type)
    }
    func decode(_ type: String.Type) throws -> String {
        if case .string(let s) = value { return s }
        throw decoder.mismatch(type)
    }
    func decode(_ type: Double.Type) throws -> Double {
        switch value {
        case .float(let d): return d
        case .int(let i): return Double(i)
        case .uint(let u): return Double(u)
        default: throw decoder.mismatch(type)
        }
    }
    func decode(_ type: Float.Type) throws -> Float { Float(try decode(Double.self)) }

    private func integer<T: FixedWidthInteger>(_ type: T.Type) throws -> T {
        let out: T?
        switch value {
        case .int(let i): out = T(exactly: i)
        case .uint(let u): out = T(exactly: u)
        case .float(let d): out = T(exactly: d)
        default: throw decoder.mismatch(type)
        }
        guard let v = out else {
            throw DecodingError.dataCorrupted(
                .init(codingPath: codingPath, debugDescription: "\(value) does not fit \(type)"))
        }
        return v
    }
    func decode(_ type: Int.Type) throws -> Int { try integer(type) }
    func decode(_ type: Int8.Type) throws -> Int8 { try integer(type) }
    func decode(_ type: Int16.Type) throws -> Int16 { try integer(type) }
    func decode(_ type: Int32.Type) throws -> Int32 { try integer(type) }
    func decode(_ type: Int64.Type) throws -> Int64 { try integer(type) }
    func decode(_ type: UInt.Type) throws -> UInt { try integer(type) }
    func decode(_ type: UInt8.Type) throws -> UInt8 { try integer(type) }
    func decode(_ type: UInt16.Type) throws -> UInt16 { try integer(type) }
    func decode(_ type: UInt32.Type) throws -> UInt32 { try integer(type) }
    func decode(_ type: UInt64.Type) throws -> UInt64 { try integer(type) }
    func decode<T: Decodable>(_ type: T.Type) throws -> T { try decoder.decodeValue(type) }
}

private struct _MPKeyed<Key: CodingKey>: KeyedDecodingContainerProtocol {
    let dict: [String: MessagePackValue]
    let codingPath: [CodingKey]
    let userInfo: [CodingUserInfoKey: Any]

    var allKeys: [Key] { dict.keys.compactMap { Key(stringValue: $0) } }
    func contains(_ key: Key) -> Bool { dict[key.stringValue] != nil }

    private func child(_ key: Key) throws -> _MPDecoder {
        guard let v = dict[key.stringValue] else {
            throw DecodingError.keyNotFound(
                key, .init(codingPath: codingPath, debugDescription: "no value for \(key.stringValue)"))
        }
        return _MPDecoder(value: v, codingPath: codingPath + [key], userInfo: userInfo)
    }

    func decodeNil(forKey key: Key) throws -> Bool {
        guard let v = dict[key.stringValue] else { return true }
        if case .nil_ = v { return true }
        return false
    }
    func decode<T: Decodable>(_ type: T.Type, forKey key: Key) throws -> T {
        try child(key).decodeValue(type)
    }
    func nestedContainer<NestedKey: CodingKey>(
        keyedBy type: NestedKey.Type, forKey key: Key
    ) throws -> KeyedDecodingContainer<NestedKey> {
        try child(key).container(keyedBy: type)
    }
    func nestedUnkeyedContainer(forKey key: Key) throws -> UnkeyedDecodingContainer {
        try child(key).unkeyedContainer()
    }
    func superDecoder() throws -> Decoder { try superDecoder(forKey: Key(stringValue: "super")!) }
    func superDecoder(forKey key: Key) throws -> Decoder { try child(key) }
}

private struct _MPUnkeyed: UnkeyedDecodingContainer {
    let items: [MessagePackValue]
    let codingPath: [CodingKey]
    let userInfo: [CodingUserInfoKey: Any]
    var currentIndex = 0

    init(items: [MessagePackValue], codingPath: [CodingKey], userInfo: [CodingUserInfoKey: Any]) {
        self.items = items
        self.codingPath = codingPath
        self.userInfo = userInfo
    }

    var count: Int? { items.count }
    var isAtEnd: Bool { currentIndex >= items.count }

    private mutating func next() throws -> _MPDecoder {
        guard !isAtEnd else {
            throw DecodingError.valueNotFound(
                Any.self, .init(codingPath: codingPath, debugDescription: "array ended"))
        }
        defer { currentIndex += 1 }
        return _MPDecoder(
            value: items[currentIndex],
            codingPath: codingPath + [_MPKey(intValue: currentIndex)],
            userInfo: userInfo)
    }

    mutating func decodeNil() throws -> Bool {
        guard !isAtEnd else { return false }
        if case .nil_ = items[currentIndex] {
            currentIndex += 1
            return true
        }
        return false
    }
    mutating func decode<T: Decodable>(_ type: T.Type) throws -> T { try next().decodeValue(type) }
    mutating func nestedContainer<NestedKey: CodingKey>(
        keyedBy type: NestedKey.Type
    ) throws -> KeyedDecodingContainer<NestedKey> {
        try next().container(keyedBy: type)
    }
    mutating func nestedUnkeyedContainer() throws -> UnkeyedDecodingContainer {
        try next().unkeyedContainer()
    }
    mutating func superDecoder() throws -> Decoder { try next() }
}

// MARK: - Encoder

/// Encodes an `Encodable` value as MessagePack. Keyed types become maps with
/// field names.
public struct MessagePackEncoder {
    public var userInfo: [CodingUserInfoKey: Any] = [:]

    public init() {}

    public func encode<T: Encodable>(_ value: T) throws -> Data {
        MessagePackWriter.write(try encodeValue(value))
    }

    public func encodeValue<T: Encodable>(_ value: T) throws -> MessagePackValue {
        let root = _MPEncoder(codingPath: [], userInfo: userInfo)
        return try root.box(value)
    }
}

/// A container the encoder fills in place and reads back at the end.
private final class _MPNode {
    enum Kind { case pending, value(MessagePackValue), map([(String, _MPNode)]), array([_MPNode]) }
    var kind: Kind = .pending

    func resolve() -> MessagePackValue {
        switch kind {
        case .pending: return .nil_
        case .value(let v): return v
        case .map(let pairs): return .map(pairs.map { (.string($0.0), $0.1.resolve()) })
        case .array(let items): return .array(items.map { $0.resolve() })
        }
    }
}

private final class _MPEncoder: Encoder {
    let codingPath: [CodingKey]
    let userInfo: [CodingUserInfoKey: Any]
    let node = _MPNode()

    init(codingPath: [CodingKey], userInfo: [CodingUserInfoKey: Any]) {
        self.codingPath = codingPath
        self.userInfo = userInfo
    }

    func box<T: Encodable>(_ value: T) throws -> MessagePackValue {
        if let data = value as? Data { return .binary(data) }
        try value.encode(to: self)
        return node.resolve()
    }

    func container<Key: CodingKey>(keyedBy type: Key.Type) -> KeyedEncodingContainer<Key> {
        if case .map = node.kind {} else { node.kind = .map([]) }
        return KeyedEncodingContainer(_MPKeyedEnc<Key>(node: node, codingPath: codingPath, userInfo: userInfo))
    }
    func unkeyedContainer() -> UnkeyedEncodingContainer {
        if case .array = node.kind {} else { node.kind = .array([]) }
        return _MPUnkeyedEnc(node: node, codingPath: codingPath, userInfo: userInfo)
    }
    func singleValueContainer() -> SingleValueEncodingContainer {
        _MPSingleEnc(encoder: self)
    }
}

private func encodeChild<T: Encodable>(
    _ value: T, codingPath: [CodingKey], userInfo: [CodingUserInfoKey: Any]
) throws -> _MPNode {
    let child = _MPEncoder(codingPath: codingPath, userInfo: userInfo)
    let v = try child.box(value)
    let n = _MPNode()
    n.kind = .value(v)
    return n
}

private struct _MPSingleEnc: SingleValueEncodingContainer {
    let encoder: _MPEncoder
    var codingPath: [CodingKey] { encoder.codingPath }
    private func set(_ v: MessagePackValue) { encoder.node.kind = .value(v) }

    mutating func encodeNil() throws { set(.nil_) }
    mutating func encode(_ value: Bool) throws { set(.bool(value)) }
    mutating func encode(_ value: String) throws { set(.string(value)) }
    mutating func encode(_ value: Double) throws { set(.float(value)) }
    mutating func encode(_ value: Float) throws { set(.float(Double(value))) }
    mutating func encode(_ value: Int) throws { set(.int(Int64(value))) }
    mutating func encode(_ value: Int8) throws { set(.int(Int64(value))) }
    mutating func encode(_ value: Int16) throws { set(.int(Int64(value))) }
    mutating func encode(_ value: Int32) throws { set(.int(Int64(value))) }
    mutating func encode(_ value: Int64) throws { set(.int(value)) }
    mutating func encode(_ value: UInt) throws { set(.uint(UInt64(value))) }
    mutating func encode(_ value: UInt8) throws { set(.uint(UInt64(value))) }
    mutating func encode(_ value: UInt16) throws { set(.uint(UInt64(value))) }
    mutating func encode(_ value: UInt32) throws { set(.uint(UInt64(value))) }
    mutating func encode(_ value: UInt64) throws { set(.uint(value)) }
    mutating func encode<T: Encodable>(_ value: T) throws {
        set(try encoder.box(value))
    }
}

private struct _MPKeyedEnc<Key: CodingKey>: KeyedEncodingContainerProtocol {
    let node: _MPNode
    let codingPath: [CodingKey]
    let userInfo: [CodingUserInfoKey: Any]

    private func put(_ key: Key, _ child: _MPNode) {
        guard case .map(var pairs) = node.kind else { return }
        if let i = pairs.firstIndex(where: { $0.0 == key.stringValue }) {
            pairs[i] = (key.stringValue, child)
        } else {
            pairs.append((key.stringValue, child))
        }
        node.kind = .map(pairs)
    }

    mutating func encodeNil(forKey key: Key) throws {
        let n = _MPNode()
        n.kind = .value(.nil_)
        put(key, n)
    }
    mutating func encode<T: Encodable>(_ value: T, forKey key: Key) throws {
        put(key, try encodeChild(value, codingPath: codingPath + [key], userInfo: userInfo))
    }
    mutating func nestedContainer<NestedKey: CodingKey>(
        keyedBy keyType: NestedKey.Type, forKey key: Key
    ) -> KeyedEncodingContainer<NestedKey> {
        let n = _MPNode()
        n.kind = .map([])
        put(key, n)
        return KeyedEncodingContainer(
            _MPKeyedEnc<NestedKey>(node: n, codingPath: codingPath + [key], userInfo: userInfo))
    }
    mutating func nestedUnkeyedContainer(forKey key: Key) -> UnkeyedEncodingContainer {
        let n = _MPNode()
        n.kind = .array([])
        put(key, n)
        return _MPUnkeyedEnc(node: n, codingPath: codingPath + [key], userInfo: userInfo)
    }
    mutating func superEncoder() -> Encoder { superEncoder(forKey: Key(stringValue: "super")!) }
    mutating func superEncoder(forKey key: Key) -> Encoder {
        let child = _MPEncoder(codingPath: codingPath + [key], userInfo: userInfo)
        put(key, child.node)
        return child
    }
}

private struct _MPUnkeyedEnc: UnkeyedEncodingContainer {
    let node: _MPNode
    let codingPath: [CodingKey]
    let userInfo: [CodingUserInfoKey: Any]

    var count: Int {
        if case .array(let items) = node.kind { return items.count }
        return 0
    }

    private func append(_ child: _MPNode) {
        guard case .array(var items) = node.kind else { return }
        items.append(child)
        node.kind = .array(items)
    }

    mutating func encodeNil() throws {
        let n = _MPNode()
        n.kind = .value(.nil_)
        append(n)
    }
    mutating func encode<T: Encodable>(_ value: T) throws {
        append(try encodeChild(value, codingPath: codingPath + [_MPKey(intValue: count)], userInfo: userInfo))
    }
    mutating func nestedContainer<NestedKey: CodingKey>(
        keyedBy keyType: NestedKey.Type
    ) -> KeyedEncodingContainer<NestedKey> {
        let n = _MPNode()
        n.kind = .map([])
        append(n)
        return KeyedEncodingContainer(
            _MPKeyedEnc<NestedKey>(node: n, codingPath: codingPath, userInfo: userInfo))
    }
    mutating func nestedUnkeyedContainer() -> UnkeyedEncodingContainer {
        let n = _MPNode()
        n.kind = .array([])
        append(n)
        return _MPUnkeyedEnc(node: n, codingPath: codingPath, userInfo: userInfo)
    }
    mutating func superEncoder() -> Encoder {
        let child = _MPEncoder(codingPath: codingPath, userInfo: userInfo)
        append(child.node)
        return child
    }
}
