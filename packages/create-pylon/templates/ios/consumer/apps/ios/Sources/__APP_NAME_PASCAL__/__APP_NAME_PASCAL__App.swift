import SwiftUI

@main
struct __APP_NAME_PASCAL__App: App {
	@StateObject private var app = AppModel()

	var body: some Scene {
		WindowGroup {
			RootView()
				.environmentObject(app)
				.task { await app.boot() }
		}
	}
}
