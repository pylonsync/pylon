import SwiftUI
import PylonClient

@main
struct __APP_NAME_PASCAL__App: App {
	@StateObject private var session = AppSession()

	var body: some Scene {
		Window("__APP_NAME__", id: "main") {
			ChatRootView()
				.environmentObject(session)
				.frame(minWidth: 700, minHeight: 500)
		}
		.windowResizability(.contentMinSize)
	}
}

@MainActor
final class AppSession: ObservableObject {
	let pylon: PylonClient
	@Published var authorName: String

	init() {
		let baseURLString = ProcessInfo.processInfo.environment["PYLON_BASE_URL"]
			?? "http://localhost:4321"
		guard let url = URL(string: baseURLString) else {
			fatalError("Invalid PYLON_BASE_URL: \(baseURLString)")
		}
		self.pylon = PylonClient(baseURL: url, appName: "__APP_NAME_SNAKE__")
		self.authorName = UserDefaults.standard.string(forKey: "authorName") ?? "anonymous"
	}

	func setAuthorName(_ name: String) {
		authorName = name
		UserDefaults.standard.set(name, forKey: "authorName")
	}
}
