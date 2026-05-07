import SwiftUI
import PylonClient

@main
struct __APP_NAME_PASCAL__App: App {
	@StateObject private var session = AppSession()

	var body: some Scene {
		WindowGroup {
			TodoListView()
				.environmentObject(session)
		}
	}
}

@MainActor
final class AppSession: ObservableObject {
	let pylon: PylonClient

	init() {
		let baseURLString = ProcessInfo.processInfo.environment["PYLON_BASE_URL"]
			?? "http://localhost:4321"
		guard let url = URL(string: baseURLString) else {
			fatalError("Invalid PYLON_BASE_URL: \(baseURLString)")
		}
		self.pylon = PylonClient(baseURL: url, appName: "__APP_NAME_SNAKE__")
	}
}
