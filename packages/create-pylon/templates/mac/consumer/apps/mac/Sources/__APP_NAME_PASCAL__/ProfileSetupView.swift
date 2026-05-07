import SwiftUI
import PylonClient
import PylonSync

struct ProfileSetupView: View {
	@EnvironmentObject var session: AppSession
	let engine: SyncEngine
	let existingHandles: [String]

	@State private var handle = ""
	@State private var displayName = ""
	@State private var bio = ""
	@State private var saving = false
	@State private var errorMessage: String?

	var body: some View {
		NavigationStack {
			Form {
				Section("Set up your profile") {
					TextField("handle (lowercase, 2–20)", text: $handle)
						.autocorrectionDisabled()
						.textInputAutocapitalization(.never)
					TextField("Display name", text: $displayName)
					TextField("Bio (optional)", text: $bio)
				}
				if let errorMessage {
					Section {
						Text(errorMessage)
							.foregroundStyle(.red)
							.font(.caption)
					}
				}
				Section {
					Button(saving ? "Saving…" : "Save") {
						Task { await save() }
					}
					.disabled(saving
						|| handle.trimmingCharacters(in: .whitespaces).isEmpty
						|| displayName.trimmingCharacters(in: .whitespaces).isEmpty)
				}
			}
			.navigationTitle("__APP_NAME__")
		}
	}

	private func save() async {
		saving = true
		defer { saving = false }
		let lower = handle.trimmingCharacters(in: .whitespaces).lowercased()
		if existingHandles.contains(lower) {
			errorMessage = "@\(lower) is taken"
			return
		}
		do {
			let profile: Profile = try await session.client.callFn(
				"upsertProfile",
				args: UpsertProfileArgs(
					handle: lower,
					displayName: displayName.trimmingCharacters(in: .whitespaces),
					bio: bio.trimmingCharacters(in: .whitespaces),
				),
			)
			session.setMyProfileId(profile.id)
		} catch {
			errorMessage = "Save failed: \(error.localizedDescription)"
		}
	}
}
