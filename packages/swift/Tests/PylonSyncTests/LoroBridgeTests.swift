import XCTest
@testable import PylonSync

final class LoroBridgeTests: XCTestCase {

    func testRoundTripEncodeDecode() {
        let payload = Data([0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02])
        let frame = PylonCrdtFrame(kind: .snapshot, entity: "Doc", rowId: "row_abc", payload: payload)
        let encoded = PylonCrdtWire.encode(frame)
        let decoded = PylonCrdtWire.decode(encoded)
        XCTAssertNotNil(decoded)
        XCTAssertEqual(decoded?.kind, .snapshot)
        XCTAssertEqual(decoded?.entity, "Doc")
        XCTAssertEqual(decoded?.rowId, "row_abc")
        XCTAssertEqual(decoded?.payload, payload)
    }

    func testTruncatedFrameReturnsNil() {
        // 5 bytes is the minimum header; anything less is invalid.
        let truncated = Data([0x10, 0x00, 0x05, 0x00])
        XCTAssertNil(PylonCrdtWire.decode(truncated))
    }

    func testUnknownKindReturnsNil() {
        // Type byte 0xFF isn't a defined frame kind.
        var bytes = Data([0xFF])
        bytes.append(contentsOf: [0x00, 0x03])  // entity_len = 3
        bytes.append(contentsOf: "Doc".utf8)
        bytes.append(contentsOf: [0x00, 0x01])  // row_id_len = 1
        bytes.append(contentsOf: "x".utf8)
        XCTAssertNil(PylonCrdtWire.decode(bytes))
    }

    func testUpdateKindMatchesWireProtocol() {
        // 0x11 = update; matches packages/loro/src/wire.ts CRDT_FRAME_UPDATE.
        let frame = PylonCrdtFrame(kind: .update, entity: "E", rowId: "r", payload: Data([0x42]))
        let encoded = PylonCrdtWire.encode(frame)
        XCTAssertEqual(encoded.first, 0x11)
        XCTAssertEqual(PylonCrdtFrame.Kind.snapshot.rawValue, 0x10)
    }

    func testRoundTripWithLargeEntityAndRowId() {
        // Stress with multi-byte UTF-8 in both name fields.
        let entity = "Дом"
        let rowId = "🚀-row-§§§"
        let payload = Data(repeating: 0xAA, count: 1024)
        let frame = PylonCrdtFrame(kind: .snapshot, entity: entity, rowId: rowId, payload: payload)
        let encoded = PylonCrdtWire.encode(frame)
        let decoded = PylonCrdtWire.decode(encoded)
        XCTAssertEqual(decoded?.entity, entity)
        XCTAssertEqual(decoded?.rowId, rowId)
        XCTAssertEqual(decoded?.payload.count, 1024)
    }
}
