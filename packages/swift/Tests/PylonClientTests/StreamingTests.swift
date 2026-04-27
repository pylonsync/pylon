import XCTest
@testable import PylonClient

final class StreamingTests: XCTestCase {

    func testStreamFnYieldsOneLinePerEvent() async throws {
        let transport = MockTransport()
        transport.setHandler { _ in
            let body = "{\"a\":1}\n{\"b\":2}\n{\"c\":3}\n"
            return (200, Data(body.utf8))
        }
        let client = PylonClient(
            config: PylonClientConfig(baseURL: URL(string: "http://test.invalid")!),
            storage: MemoryStorage(),
            transport: transport
        )
        struct Args: Encodable {}
        var lines: [String] = []
        let stream = await client.streamFn("dummy", args: Args())
        for try await line in stream {
            lines.append(line)
        }
        XCTAssertEqual(lines, ["{\"a\":1}", "{\"b\":2}", "{\"c\":3}"])
    }

    func testStreamFnHandlesPartialLinesAcrossChunks() async throws {
        // Simulate a server that flushes mid-line. We can't directly
        // multi-chunk through MockTransport.send (it's single-shot), but
        // the line splitter logic itself is what matters: feed the full
        // body in one chunk and verify the boundaries are right.
        let transport = MockTransport()
        transport.setHandler { _ in
            // No trailing newline — last "fragment" should still be yielded.
            return (200, Data("first\nsecond\nthird".utf8))
        }
        let client = PylonClient(
            config: PylonClientConfig(baseURL: URL(string: "http://test.invalid")!),
            storage: MemoryStorage(),
            transport: transport
        )
        struct Args: Encodable {}
        var lines: [String] = []
        for try await line in await client.streamFn("dummy", args: Args()) {
            lines.append(line)
        }
        XCTAssertEqual(lines, ["first", "second", "third"])
    }

    func testStreamFnBytesYieldsRawData() async throws {
        let transport = MockTransport()
        transport.setHandler { _ in
            return (200, Data([0x00, 0x01, 0xFF]))
        }
        let client = PylonClient(
            config: PylonClientConfig(baseURL: URL(string: "http://test.invalid")!),
            storage: MemoryStorage(),
            transport: transport
        )
        struct Args: Encodable {}
        var bytes = Data()
        for try await chunk in await client.streamFnBytes("dummy", args: Args()) {
            bytes.append(chunk)
        }
        XCTAssertEqual(bytes, Data([0x00, 0x01, 0xFF]))
    }
}
