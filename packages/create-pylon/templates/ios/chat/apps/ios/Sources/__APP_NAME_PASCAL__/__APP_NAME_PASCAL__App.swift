import SwiftUI
import PylonClient
import PylonSync

@main
struct __APP_NAME_PASCAL__App: App {
	@StateObject private var session = AppSession()

	var body: some Scene {
		WindowGroup {
			Group {
				if let engine = session.engine {
					ChatRootView(engine: engine)
						.environmentObject(session)
				} else {
					ProgressView()
						.frame(maxWidth: .infinity, maxHeight: .infinity)
				}
			}
			.task { await session.bootIfNeeded() }
		}
	}
}

@MainActor
final class AppSession: ObservableObject {
	let client: PylonClient
	@Published private(set) var engine: SyncEngine?
	@Published var authorName: String

	init() {
		let baseURLString = ProcessInfo.processInfo.environment["PYLON_BASE_URL"]
			?? "http://localhost:4321"
		guard let url = URL(string: baseURLString) else {
			fatalError("Invalid PYLON_BASE_URL: \(baseURLString)")
		}
		self.client = PylonClient(baseURL: url, appName: "__APP_NAME_SNAKE__")
		self.authorName = UserDefaults.standard.string(forKey: "authorName") ?? "anonymous"
	}

	/// Construct + start the SyncEngine. Idempotent — safe to call from
	/// .task on every scene mount.
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

	func setAuthorName(_ name: String) {
		authorName = name
		UserDefaults.standard.set(name, forKey: "authorName")
	}
}
