import PhotosUI
import SwiftUI

/// Create the user's profile after sign-up, or edit it later.
struct ProfileEditorView: View {
	enum Mode { case create, edit }

	@EnvironmentObject private var app: AppModel
	@EnvironmentObject private var social: SocialStore
	@Environment(\.dismiss) private var dismiss

	let mode: Mode

	@State private var displayName = ""
	@State private var handle = ""
	@State private var bio = ""
	@State private var pickedItem: PhotosPickerItem?
	@State private var pickedImage: UIImage?
	@State private var saving = false
	@State private var errorMessage: String?
	@State private var loaded = false

	private let bioLimit = 150

	var body: some View {
		ScrollView {
			VStack(spacing: 24) {
				avatarPicker
				VStack(spacing: 12) {
					labeled("Name") {
						TextField("Your name", text: $displayName)
							.textContentType(.name)
					}
					labeled("Username") {
						HStack(spacing: 2) {
							Text("@").foregroundStyle(.secondary)
							TextField("username", text: $handle)
								.textContentType(.username)
								.textInputAutocapitalization(.never)
								.autocorrectionDisabled()
								.onChange(of: handle) { _, value in
									// The server accepts 2 to 24 of a-z, 0-9, ".", "_".
									let allowed = Set("abcdefghijklmnopqrstuvwxyz0123456789._")
									let cleaned = String(value.lowercased().filter { allowed.contains($0) }.prefix(24))
									if cleaned != value { handle = cleaned }
								}
						}
					}
					labeled("Bio") {
						TextField("A line about you", text: $bio, axis: .vertical)
							.lineLimit(1...4)
							.padding(.vertical, 14)
					} trailing: {
						Text("\(bio.count)/\(bioLimit)")
							.font(.caption.monospacedDigit())
							.foregroundStyle(bio.count > bioLimit ? .red : .secondary)
					}
				}
				if let errorMessage {
					Text(errorMessage)
						.font(.footnote)
						.foregroundStyle(.red)
						.frame(maxWidth: .infinity, alignment: .leading)
				}
				if mode == .create {
					Button("Continue", action: save)
						.buttonStyle(PrimaryButtonStyle(isLoading: saving))
						.disabled(!canSave || saving)
						.opacity(canSave ? 1 : 0.6)
					Button("Sign out") { Task { await app.signOut() } }
						.font(.footnote)
						.foregroundStyle(.secondary)
				}
			}
			.padding(20)
		}
		.scrollDismissesKeyboard(.interactively)
		.navigationTitle(mode == .create ? "Create your profile" : "Edit profile")
		.navigationBarTitleDisplayMode(.inline)
		.toolbar {
			if mode == .edit {
				ToolbarItem(placement: .cancellationAction) {
					Button("Cancel") { dismiss() }
				}
				ToolbarItem(placement: .confirmationAction) {
					if saving {
						ProgressView()
					} else {
						Button("Done", action: save)
							.fontWeight(.semibold)
							.disabled(!canSave)
					}
				}
			}
		}
		.onAppear(perform: fillFromProfile)
		.onChange(of: pickedItem) { _, item in
			Task { await loadPicked(item) }
		}
	}

	private var avatarPicker: some View {
		let me = social.me
		return PhotosPicker(selection: $pickedItem, matching: .images) {
			VStack(spacing: 10) {
				ZStack(alignment: .bottomTrailing) {
					Group {
						if let pickedImage {
							Image(uiImage: pickedImage)
								.resizable()
								.scaledToFill()
								.frame(width: 96, height: 96)
								.clipShape(Circle())
						} else if let me, me.avatarUrl != nil {
							AvatarView(profile: me, size: 96)
						} else {
							Circle()
								.fill(Theme.fieldFill)
								.frame(width: 96, height: 96)
								.overlay {
									Image(systemName: "person.fill")
										.font(.system(size: 40))
										.foregroundStyle(.tertiary)
								}
						}
					}
					Image(systemName: "camera.fill")
						.font(.system(size: 13, weight: .semibold))
						.foregroundStyle(.white)
						.frame(width: 30, height: 30)
						.background(Theme.accent, in: Circle())
						.overlay(Circle().stroke(Color(uiColor: .systemBackground), lineWidth: 3))
				}
				Text(pickedImage == nil && me?.avatarUrl == nil ? "Add a profile photo" : "Change photo")
					.font(.subheadline.weight(.semibold))
					.foregroundStyle(Theme.accent)
			}
		}
		.buttonStyle(.plain)
		.padding(.top, 8)
	}

	private func labeled<Content: View, Trailing: View>(
		_ title: String,
		@ViewBuilder content: () -> Content,
		@ViewBuilder trailing: () -> Trailing = { EmptyView() }
	) -> some View {
		VStack(alignment: .leading, spacing: 6) {
			HStack {
				Text(title)
					.font(.footnote.weight(.semibold))
					.foregroundStyle(.secondary)
				Spacer()
				trailing()
			}
			content()
				.fieldStyle()
		}
	}

	private var canSave: Bool {
		!displayName.trimmingCharacters(in: .whitespaces).isEmpty
			&& handle.count >= 2
			&& bio.count <= bioLimit
	}

	private func fillFromProfile() {
		guard !loaded else { return }
		loaded = true
		if let me = social.me {
			displayName = me.displayName
			handle = me.handle
			bio = me.bio ?? ""
		}
	}

	private func loadPicked(_ item: PhotosPickerItem?) async {
		guard let item,
		      let data = try? await item.loadTransferable(type: Data.self),
		      let image = UIImage(data: data) else { return }
		pickedImage = image
	}

	private func save() {
		guard canSave, !saving else { return }
		saving = true
		errorMessage = nil
		Task {
			defer { saving = false }
			do {
				let avatar = pickedImage?.jpegForUpload(maxPixels: 600)
				let profile = try await social.saveProfile(
					handle: handle,
					displayName: displayName.trimmingCharacters(in: .whitespaces),
					bio: bio.trimmingCharacters(in: .whitespacesAndNewlines),
					avatarJPEG: avatar
				)
				if mode == .create {
					app.profileCreated(profile)
				} else {
					dismiss()
				}
			} catch {
				errorMessage = friendlyMessage(error)
			}
		}
	}
}
