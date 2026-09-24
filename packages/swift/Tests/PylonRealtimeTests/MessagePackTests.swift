import XCTest
@testable import PylonRealtime

final class MessagePackTests: XCTestCase {
    struct Player: Codable, Equatable {
        let id: String
        let x: Double
        let hp: Int
        let tags: [String]
        let guild: String?
    }

    struct Zone: Codable, Equatable {
        let tick: UInt64
        let players: [Player]
        let blob: Data
    }

    func testCodableRoundTrip() throws {
        let zone = Zone(
            tick: 1 << 40,
            players: [
                Player(id: "a", x: -1.5, hp: 100, tags: ["tank"], guild: nil),
                Player(id: "b", x: 3, hp: -7, tags: [], guild: "north"),
            ],
            blob: Data([0, 1, 2, 255])
        )
        let bytes = try MessagePackEncoder().encode(zone)
        XCTAssertEqual(try MessagePackDecoder().decode(Zone.self, from: bytes), zone)
    }

    func testStructsEncodeAsNamedMaps() throws {
        struct Env: Encodable {
            let input: Int
            let client_seq: UInt64
        }
        let value = try MessagePackEncoder().encodeValue(Env(input: 5, client_seq: 9))
        XCTAssertEqual(value, .map([(.string("input"), .int(5)), (.string("client_seq"), .uint(9))]))
    }

    func testReadsEveryIntegerWidth() throws {
        // uint8, uint16, uint32, uint64, int8, int16, int32, int64, negative fixint
        let cases: [([UInt8], MessagePackValue)] = [
            ([0xcc, 0xff], .uint(255)),
            ([0xcd, 0x01, 0x00], .uint(256)),
            ([0xce, 0x00, 0x01, 0x00, 0x00], .uint(65536)),
            ([0xcf, 0, 0, 0, 1, 0, 0, 0, 0], .uint(1 << 32)),
            ([0xd0, 0x80], .int(-128)),
            ([0xd1, 0xff, 0x00], .int(-256)),
            ([0xd2, 0xff, 0xff, 0x00, 0x00], .int(-65536)),
            ([0xd3, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0], .int(-(1 << 32))),
            ([0xff], .int(-1)),
        ]
        for (bytes, want) in cases {
            XCTAssertEqual(try MessagePackReader.read(Data(bytes)), want, "\(bytes)")
            XCTAssertEqual(try MessagePackReader.read(MessagePackWriter.write(want)), want)
        }
    }

    func testRejectsTruncatedInput() {
        XCTAssertThrowsError(try MessagePackReader.read(Data([0xcd, 0x01])))
        XCTAssertThrowsError(try MessagePackReader.read(Data([0x92, 0x01])))
    }

    func testOutOfRangeIntegerFails() {
        let bytes = MessagePackWriter.write(.int(-1))
        XCTAssertThrowsError(try MessagePackDecoder().decode(UInt8.self, from: bytes))
    }

    func testParsesAVersion2Frame() throws {
        var data = Data([ShardWire.Kind.snapshot.rawValue, ShardWire.Codec.messagePack.rawValue])
        data.append(contentsOf: [0, 0, 0, 0, 0, 0, 0, 42])  // tick
        data.append(contentsOf: [0, 0, 0, 0, 0, 0, 0, 7])  // ack
        data.append(try MessagePackEncoder().encode(["hp": 5]))
        let frame = try ShardWire.parse(data)
        XCTAssertEqual(frame.tick, 42)
        XCTAssertEqual(frame.ack, 7)
        let state = try ShardWire.decode([String: Int].self, codec: frame.codec, payload: frame.payload)
        XCTAssertEqual(state, ["hp": 5])
        XCTAssertThrowsError(try ShardWire.parse(Data([1, 2, 3])))
    }
}
