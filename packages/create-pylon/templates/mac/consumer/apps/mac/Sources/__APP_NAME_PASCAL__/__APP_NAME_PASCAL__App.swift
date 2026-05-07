import SwiftUI
import PylonClient
import PylonSync

@main
struct __APP_NAME_PASCAL__App: App {
	@StateObject private var session = AppSession()

	var body: some Scene {
		Window("__APP_NAME__", id: "main") {
			Group {
				if let engine = session.engine {
					RootView(engine: engine)
						.environmentObject(session)
						.frame(minWidth: 480, minHeight: 600)
				} else {
					ProgressView()
						.frame(maxWidth: .infinity, maxHeight: .infinity)
				}
			}
			.task { await session.bootIfNeeded() }
		}
		.windowResizability(.contentMinSize)
	}
}

@MainActor
final class AppSession: ObservableObject {
	let client: PylonClient
	@Published private(set) var engine: SyncEngine?
	@Published var myProfileId: String?

	init() {
		let baseURLString = ProcessInfo.processInfo.environment["PYLON_BASE_URL"]
			?? "http://localhost:4321"
		guard let url = URL(string: baseURLString) else {
			fatalError("Invalid PYLON_BASE_URL: \(baseURLString)")
		}
		self.client = PylonClient(baseURL: url, appName: "__APP_NAME_SNAKE__")
		self.myProfileId = UserDefaults.standard.string(forKey: "myProfileId")
	}

	func bootIfNeeded() async {
		guard engine == nil else { return }
		let baseURLString = ProcessInfo.processInfo.environment["PYLON_BASE_URL"]
			?? "http://localhost:4321"
		guard let url = URL(string: baseURLString) else { return }
		let config = SyncEngineConfig(baseURL: url, appName: "__APP_NAME_SNAKE__")
		let engine = await SyncEngine(config: config, client: client)
		await engine.start()
		self.engine = engine
	}

	func setMyProfileId(_ id: String?) {
		myProfileId = id
		if let id { UserDefaults.standard.set(id, forKey: "myProfileId") }
		else { UserDefaults.standard.removeObject(forKey: "myProfileId") }
	}
}
