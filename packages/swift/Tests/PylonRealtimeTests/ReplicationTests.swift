import XCTest
@testable import PylonRealtime

/// The Swift decoder against frames the Rust encoder wrote
/// (packages/realtime/src/replication.fixtures.json, checked against the
/// encoder by crates/realtime/tests/replication_fixtures.rs).
final class ReplicationTests: XCTestCase {
    struct Fixtures: Decodable {
        struct Step: Decodable {
            let frame: String
            let precision: Double
            let table: [Row]
        }
        struct Row: Decodable, Equatable {
            let id: UInt64
            let q: [Int64]
            let components: [String: String]
        }
        let frames: [Step]
    }

    static func bytes(_ hex: String) -> Data {
        var out = Data()
        var i = hex.startIndex
        while i < hex.endIndex {
            let j = hex.index(i, offsetBy: 2)
            out.append(UInt8(hex[i..<j], radix: 16)!)
            i = j
        }
        return out
    }

    static func hex(_ d: Data) -> String {
        d.map { String(format: "%02x", $0) }.joined()
    }

    func testTheRustEncodersFramesGiveTheSameTables() throws {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .appendingPathComponent("../../../realtime/src/replication.fixtures.json")
        let fixtures = try JSONDecoder().decode(Fixtures.self, from: Data(contentsOf: url))
        var table = EntityTable()
        for (i, step) in fixtures.frames.enumerated() {
            try table.apply(Self.bytes(step.frame))
            let rows = table.entities.values
                .sorted { $0.id < $1.id }
                .map { e in
                    Fixtures.Row(
                        id: e.id,
                        q: e.q,
                        components: Dictionary(uniqueKeysWithValues: e.components.map { (String($0.key), Self.hex($0.value)) })
                    )
                }
            XCTAssertEqual(rows, step.table, "after frame \(i)")
            for e in table.entities.values {
                XCTAssertEqual(e.position.x, Double(e.q[0]) * step.precision, accuracy: 1e-6)
            }
        }
    }

    func testHostileFramesAreRefused() {
        func head(_ rest: [UInt8]) -> Data {
            var d = Data([1, 0])
            withUnsafeBytes(of: Float(1).bitPattern.littleEndian) { d.append(contentsOf: $0) }
            d.append(contentsOf: rest)
            return d
        }
        var t = EntityTable()
        XCTAssertThrowsError(try t.apply(Data()))
        XCTAssertThrowsError(try t.apply(Data([2, 0, 0, 0, 0x80, 0x3f])))
        XCTAssertThrowsError(try t.apply(Data([1, 0, 0, 0, 0, 0])))
        XCTAssertThrowsError(try t.apply(head([0xff, 0xff, 0xff, 0xff, 0x0f])))
        XCTAssertThrowsError(try t.apply(head([0, 0, 1, 7, 0])))
        XCTAssertThrowsError(try t.apply(head([2, 5, 0, 0, 0])))
        XCTAssertThrowsError(try t.apply(head([0, 0, 0, 9])))
        XCTAssertThrowsError(try t.apply(head([0, 1, 1, 0, 0, 0, 1, 4, 50, 1])))
    }
}
