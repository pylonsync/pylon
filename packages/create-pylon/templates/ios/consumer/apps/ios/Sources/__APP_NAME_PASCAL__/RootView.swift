import SwiftUI
import PylonClient

struct RootView: View {
	@EnvironmentObject var session: AppSession
	@State private var loading = true

	var body: some View {
		Group {
			if loading {
				ProgressView()
					.frame(maxWidth: .infinity, maxHeight: .infinity)
			} else if session.me == nil {
				ProfileSetupView()
			} else {
				FeedView()
			}
		}
		.task { await load() }
	}

	private func load() async {
		do {
			session.me = try await session.pylon.callFn("myProfile", args: EmptyArgs())
		} catch {
			session.me = nil
		}
		loading = false
	}
}
