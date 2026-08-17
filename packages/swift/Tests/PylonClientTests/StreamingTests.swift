import XCTest
@testable import PylonClient

final class StreamingTests: XCTestCase {

    private func sseClient(_ transport: MockTransport) -> PylonClient {
        transport.setResponseHeaders([
            "Content-Type": "text/event-stream",
            "X-Pylon-Stream-Id": "st_abc123",
        ])
        return PylonClient(
            config: PylonClientConfig(baseURL: URL(string: "http://test.invalid")!),
            storage: MemoryStorage(),
            transport: transport
        )
    }

    func testStreamFnParsesSseFramesAndResult() async throws {
        let transport = MockTransport()
        transport.setHandler { _ in
            let body =
                "retry: 1000\n\n" +
                "id: 1\ndata: hello\n\n" +
                "id: 2\nevent: tick\ndata: {\"n\":1}\n\n" +
                "id: 3\nevent: result\ndata: {\"done\":true}\n\n"
            return (200, Data(body.utf8))
        }
        let client = sseClient(transport)
        struct Args: Encodable {}

        let ids = LockedBox<[String]>([])
        let results = LockedBox<[String]>([])
        var chunks: [String] = []
        let stream = await client.streamFn(
            "dummy", args: Args(),
            onStreamId: { ids.append($0) },
            onResult: { results.append($0) }
        )
        for try await chunk in stream {
            chunks.append(chunk)
        }
        // Data frames only — the typed `tick` frame's payload is yielded,
        // the result frame terminates without being yielded.
        XCTAssertEqual(chunks, ["hello", "{\"n\":1}"])
        XCTAssertEqual(ids.value, ["st_abc123"])
        XCTAssertEqual(results.value, ["{\"done\":true}"])
    }

    func testStreamFnRejoinsMultiLineData() async throws {
        let transport = MockTransport()
        transport.setHandler { _ in
            let body =
                "id: 1\ndata: line1\ndata: line2\n\n" +
                "id: 2\nevent: result\ndata: null\n\n"
            return (200, Data(body.utf8))
        }
        let client = sseClient(transport)
        struct Args: Encodable {}
        var chunks: [String] = []
        for try await chunk in await client.streamFn("dummy", args: Args()) {
            chunks.append(chunk)
        }
        XCTAssertEqual(chunks, ["line1\nline2"])
    }

    func testStreamFnThrowsOnErrorFrame() async throws {
        let transport = MockTransport()
        transport.setHandler { _ in
            let body =
                "id: 1\ndata: partial\n\n" +
                "id: 2\nevent: error\ndata: {\"code\":\"BOOM\",\"message\":\"handler failed\"}\n\n"
            return (200, Data(body.utf8))
        }
        let client = sseClient(transport)
        struct Args: Encodable {}
        var chunks: [String] = []
        do {
            for try await chunk in await client.streamFn("dummy", args: Args()) {
                chunks.append(chunk)
            }
            XCTFail("error frame must throw")
        } catch {
            XCTAssertTrue("\(error)".contains("handler failed"), "\(error)")
        }
        XCTAssertEqual(chunks, ["partial"])
    }

    func testNonStreamingAnswerDeliversResultWithoutChunks() async throws {
        let transport = MockTransport()
        // Default headers = application/json → plain answer path.
        transport.setHandler { _ in (200, Data("{\"ok\":true}".utf8)) }
        let client = PylonClient(
            config: PylonClientConfig(baseURL: URL(string: "http://test.invalid")!),
            storage: MemoryStorage(),
            transport: transport
        )
        struct Args: Encodable {}
        let results = LockedBox<[String]>([])
        var chunks: [String] = []
        let stream = await client.streamFn("dummy", args: Args(), onResult: { results.append($0) })
        for try await chunk in stream {
            chunks.append(chunk)
        }
        XCTAssertEqual(chunks, [])
        XCTAssertEqual(results.value, ["{\"ok\":true}"])
    }

    func testResumeStreamRequestsSinceCursor() async throws {
        let transport = MockTransport()
        transport.setResponseHeaders(["Content-Type": "text/event-stream"])
        transport.setHandler { _ in
            let body =
                "id: 5\ndata: later\n\n" +
                "id: 6\nevent: result\ndata: 42\n\n"
            return (200, Data(body.utf8))
        }
        let client = PylonClient(
            config: PylonClientConfig(baseURL: URL(string: "http://test.invalid")!),
            storage: MemoryStorage(),
            transport: transport
        )
        var chunks: [String] = []
        for try await chunk in await client.resumeStream("st_abc123", since: 4) {
            chunks.append(chunk)
        }
        XCTAssertEqual(chunks, ["later"])
        let url = transport.lastRequest()?.url?.absoluteString ?? ""
        XCTAssertTrue(url.contains("/api/fn-streams/st_abc123"), url)
        XCTAssertTrue(url.contains("since=4"), url)
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

    func testSseParserHandlesChunkSplitsHeartbeatsAndIds() {
        let parser = SseParser()
        // Frame split across three feeds, plus a heartbeat comment.
        var frames = parser.feed(Data(": hb\n\nid: ".utf8))
        XCTAssertTrue(frames.isEmpty)
        frames = parser.feed(Data("7\nev".utf8))
        XCTAssertTrue(frames.isEmpty)
        frames = parser.feed(Data("ent: tick\ndata: a\ndata: b\n\n".utf8))
        XCTAssertEqual(frames.count, 1)
        XCTAssertEqual(frames[0].seq, 7)
        XCTAssertEqual(frames[0].event, "tick")
        XCTAssertEqual(frames[0].data, "a\nb")
    }
}

/// Tiny thread-safe accumulator for @Sendable test callbacks.
final class LockedBox<T>: @unchecked Sendable {
    private let lock = NSLock()
    private var inner: T
    init(_ value: T) { self.inner = value }
    var value: T {
        lock.lock(); defer { lock.unlock() }
        return inner
    }
}

extension LockedBox where T == [String] {
    func append(_ s: String) {
        lock.lock(); defer { lock.unlock() }
        inner.append(s)
    }
}
