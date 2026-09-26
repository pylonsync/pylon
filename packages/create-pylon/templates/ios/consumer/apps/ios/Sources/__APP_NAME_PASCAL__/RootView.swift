import SwiftUI

/// Picks the screen for the session state: sign in, profile setup, or the app.
struct RootView: View {
	@EnvironmentObject private var app: AppModel

	var body: some View {
		ZStack {
			switch app.phase {
			case .launching:
				Wordmark(size: 40)
					.frame(maxWidth: .infinity, maxHeight: .infinity)
			case .signedOut:
				AuthView()
					.transition(.opacity)
			case .needsProfile:
				if let social = app.social {
					NavigationStack {
						ProfileEditorView(mode: .create)
					}
					.environmentObject(social)
					.transition(.move(edge: .trailing))
				}
			case .ready:
				if let social = app.social {
					MainTabView()
						.environmentObject(social)
						.transition(.opacity)
				}
			}
		}
		.animation(.easeInOut(duration: 0.3), value: app.phase)
	}
}
