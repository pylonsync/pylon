import SwiftUI
import PylonClient

/// SwiftUI entry point. Bootstraps a single shared PylonClient pointed
/// at the local Pylon control plane (`pylon dev`'s default port). For
/// production, override `PYLON_BASE_URL` via the build environment or
/// edit the URL inline.
@main
struct __APP_NAME_PASCAL__App: App {
	@StateObject private var session = AppSession()

	var body: some Scene {
		WindowGroup {
			ContentView()
				.environmentObject(session)
		}
	}
}

/// Shared app state. The PylonClient is held here so every view in
/// the tree gets the same instance via `@EnvironmentObject`.
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
