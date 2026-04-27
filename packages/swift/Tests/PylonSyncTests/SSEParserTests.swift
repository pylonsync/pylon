import XCTest
@testable import PylonSync

final class SSEParserTests: XCTestCase {

    func testSingleEvent() {
        var parser = SSEParser()
        let events = parser.feed("data: {\"seq\":1}\n\n")
        XCTAssertEqual(events, ["{\"seq\":1}"])
    }

    func testMultipleEventsInOneChunk() {
        var parser = SSEParser()
        let events = parser.feed("data: a\n\ndata: b\n\ndata: c\n\n")
        XCTAssertEqual(events, ["a", "b", "c"])
    }

    func testPartialEventBuffered() {
        var parser = SSEParser()
        XCTAssertEqual(parser.feed("data: he"), [])
        XCTAssertEqual(parser.feed("llo\n"), [])
        XCTAssertEqual(parser.feed("\n"), ["hello"])
    }

    func testMultilineDataAccumulates() {
        var parser = SSEParser()
        // Two data lines in one event get joined with \n.
        let events = parser.feed("data: line1\ndata: line2\n\n")
        XCTAssertEqual(events, ["line1\nline2"])
    }

    func testIgnoresNonDataLines() {
        var parser = SSEParser()
        // event:, id:, retry: lines are ignored — we only care about data:
        let events = parser.feed("event: change\nid: 42\ndata: payload\n\n")
        XCTAssertEqual(events, ["payload"])
    }

    func testNoLeadingSpaceAfterColon() {
        var parser = SSEParser()
        // Per SSE spec, the first space after the colon is optional; we
        // tolerate both.
        XCTAssertEqual(parser.feed("data:no-space\n\n"), ["no-space"])
        XCTAssertEqual(parser.feed("data: with-space\n\n"), ["with-space"])
    }
}
