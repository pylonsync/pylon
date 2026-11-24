import Foundation

// Compile with the consumer template's MediaUpload.swift.
@main
struct MediaUploadTest {
	static func main() {
		let base = URL(string: "https://app.example")!
		let cases = [
			("https://app.example/upload", true),
			("https://APP.example:443/upload", true),
			("http://app.example/upload", false),
			("https://storage.example/upload", false),
			("https://app.example:8443/upload", false),
		]
		for (url, authorized) in cases {
			let request = mediaUploadRequest(to: URL(string: url)!, from: base, token: "test")
			precondition(request.value(forHTTPHeaderField: "Authorization") == (authorized ? "Bearer test" : nil), url)
		}
	}
}
