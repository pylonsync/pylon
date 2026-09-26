import SwiftUI

/// The five tabs. The New Post tab opens the composer as a sheet and
/// keeps the current tab selected underneath it.
struct MainTabView: View {
	enum Tab: Hashable { case home, explore, compose, activity, profile }

	@EnvironmentObject private var social: SocialStore
	@State private var selection: Tab = .home
	@State private var composing = false
	@State private var feedScrollToken = 0

	var body: some View {
		TabView(selection: $selection) {
			NavigationStack { FeedView(scrollToken: feedScrollToken) { composing = true } }
				.tabItem { Label("Home", systemImage: "house") }
				.tag(Tab.home)

			NavigationStack { ExploreView() }
				.tabItem { Label("Search", systemImage: "magnifyingglass") }
				.tag(Tab.explore)

			Color.clear
				.tabItem { Label("New Post", systemImage: "plus.app") }
				.tag(Tab.compose)

			NavigationStack { ActivityView() }
				.tabItem { Label("Activity", systemImage: "heart") }
				.tag(Tab.activity)

			NavigationStack {
				if let me = social.me {
					ProfileView(profileId: me.id, isRoot: true)
				}
			}
			.tabItem { Label("Profile", systemImage: "person.crop.circle") }
			.tag(Tab.profile)
		}
		.tint(.primary)
		.onChange(of: selection) { old, new in
			if new == .compose {
				selection = old
				composing = true
			}
		}
		.sheet(isPresented: $composing) {
			ComposeView {
				selection = .home
				feedScrollToken += 1
			}
			.environmentObject(social)
		}
	}
}
