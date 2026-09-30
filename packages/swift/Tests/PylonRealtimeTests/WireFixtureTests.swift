import XCTest
import PylonClient
@testable import PylonRealtime

/// Frames and input envelopes the Rust encoders wrote
/// (packages/realtime/src/wire.fixtures.json, checked against the encoders
/// by crates/realtime/tests/wire_fixtures.rs). The TypeScript and C#
/// clients run the same file.
final class WireFixtureTests: XCTestCase {
    struct Fixtures: Decodable {
        struct Frame: Decodable {
            let frame: String
            let kind: UInt8
            let codec: UInt8
            let tick: UInt64
            let ack: UInt64
            let payload: JSONValue
        }
        struct Input: Decodable {
            let input: JSONValue
            let client_seq: UInt64
            let json: String
            let msgpack: String
        }
        let version: Int
        let headerLength: Int
        let frames: [Frame]
        let inputs: [Input]
    }

    struct Envelope: Encodable {
        let input: JSONValue
        let client_seq: UInt64
    }

    /// Equal as JSON values, with numbers compared by value (MessagePack
    /// floats decode as doubles, JSON whole numbers as ints).
    static func same(_ a: JSONValue, _ b: JSONValue) -> Bool {
        switch (a, b) {
        case (.int, .double), (.double, .int), (.int, .int), (.double, .double):
            return a.doubleValue == b.doubleValue
        case let (.array(x), .array(y)):
            return x.count == y.count && zip(x, y).allSatisfy { same($0, $1) }
        case let (.object(x), .object(y)):
            return x.count == y.count && x.allSatisfy { k, v in y[k].map { same(v, $0) } ?? false }
        default:
            return a == b
        }
    }

    func load() throws -> Fixtures {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .appendingPathComponent("../../../realtime/src/wire.fixtures.json")
        return try JSONDecoder().decode(Fixtures.self, from: Data(contentsOf: url))
    }

    func testFramesParseAndDecodeToWhatTheServerEncoded() throws {
        let fixtures = try load()
        XCTAssertEqual(fixtures.version, ShardWire.version)
        XCTAssertEqual(fixtures.headerLength, ShardWire.headerLength)
        for (i, f) in fixtures.frames.enumerated() {
            let frame = try ShardWire.parse(ReplicationTests.bytes(f.frame))
            XCTAssertEqual(frame.kind, f.kind, "frame \(i)")
            XCTAssertEqual(frame.codec, f.codec)
            XCTAssertEqual(frame.tick, f.tick)
            XCTAssertEqual(frame.ack, f.ack)
            let payload = try ShardWire.decode(JSONValue.self, codec: frame.codec, payload: frame.payload)
            XCTAssertTrue(Self.same(payload, f.payload), "frame \(i): \(payload)")
            if frame.kind == ShardWire.Kind.inputRejected.rawValue {
                let r = try ShardWire.decode(ShardInputRejection.self, codec: frame.codec, payload: frame.payload)
                XCTAssertEqual(r.code, f.payload["code"].stringValue)
                XCTAssertEqual(r.clientSeq.map { Int64($0) }, f.payload["client_seq"].intValue)
            }
            if frame.kind == ShardWire.Kind.transfer.rawValue {
                let t = try ShardWire.decode(ShardTransferNotice.self, codec: frame.codec, payload: frame.payload)
                XCTAssertEqual(t.shard, f.payload["shard"].stringValue)
                XCTAssertEqual(t.ticket, f.payload["ticket"].stringValue)
            }
        }
    }

    func testInputsEncodeToWhatTheServerDecodes() throws {
        for c in try load().inputs {
            let env = Envelope(input: c.input, client_seq: c.client_seq)
            let packed = try MessagePackEncoder().encode(env)
            let ours = try MessagePackDecoder().decode(JSONValue.self, from: packed)
            let rust = try MessagePackDecoder().decode(JSONValue.self, from: ReplicationTests.bytes(c.msgpack))
            XCTAssertTrue(Self.same(ours, rust), "\(ours) vs \(rust)")
            let text = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(env))
            let expected = try JSONDecoder().decode(JSONValue.self, from: Data(c.json.utf8))
            XCTAssertTrue(Self.same(text, expected))
        }
    }
}
