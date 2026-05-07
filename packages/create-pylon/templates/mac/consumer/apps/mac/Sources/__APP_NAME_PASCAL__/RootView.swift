import SwiftUI
import PylonClient
import PylonSync
import PylonSwiftUI

/// Watches the live Profile collection. If we have a saved profileId
/// AND a Profile row in the local store with that id, render the
/// feed; otherwise, profile setup. The profile row appears in the
/// store via the SyncEngine, so a fresh device sees its own profile
/// pop in moments after `upsertProfile` returns.
struct RootView: View {
	@EnvironmentObject var session: AppSession
	let engine: SyncEngine
	@StateObject private var profiles: PylonQuery<Profile>

	init(engine: SyncEngine) {
		self.engine = engine
		_profiles = StateObject(
			wrappedValue: PylonQuery<Profile>(engine: engine, entity: "Profile"),
		)
	}

	var body: some View {
		Group {
			if let me = currentProfile {
				FeedView(engine: engine, me: me, profiles: profiles.rows)
			} else {
				ProfileSetupView(
					engine: engine,
					existingHandles: profiles.rows.map { $0.handle },
				)
			}
		}
	}

	private var currentProfile: Profile? {
		guard let id = session.myProfileId else { return nil }
		return profiles.rows.first { $0.id == id }
	}
}
