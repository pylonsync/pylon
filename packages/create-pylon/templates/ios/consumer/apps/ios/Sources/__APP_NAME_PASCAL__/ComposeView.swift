import PhotosUI
import SwiftUI

/// New post: pick a photo from the library, write a caption, share.
struct ComposeView: View {
	@EnvironmentObject private var social: SocialStore
	@Environment(\.dismiss) private var dismiss
	let onPosted: () -> Void

	@State private var pickerShown = false
	@State private var pickedItem: PhotosPickerItem?
	@State private var image: UIImage?
	@State private var caption = ""
	@State private var posting = false
	@State private var errorMessage: String?
	@FocusState private var captionFocused: Bool

	private let captionLimit = 2200

	var body: some View {
		NavigationStack {
			ScrollView {
				VStack(alignment: .leading, spacing: 16) {
					photoArea
					if image != nil {
						HStack(alignment: .top, spacing: 12) {
							AvatarView(profile: social.me, size: 36)
							TextField("Write a caption", text: $caption, axis: .vertical)
								.lineLimit(3...8)
								.focused($captionFocused)
						}
						.padding(.horizontal, 16)
						if caption.count > captionLimit - 200 {
							Text("\(caption.count)/\(captionLimit)")
								.font(.caption.monospacedDigit())
								.foregroundStyle(caption.count > captionLimit ? .red : .secondary)
								.padding(.horizontal, 16)
						}
					}
					if let errorMessage {
						Text(errorMessage)
							.font(.footnote)
							.foregroundStyle(.red)
							.padding(.horizontal, 16)
					}
				}
			}
			.scrollDismissesKeyboard(.interactively)
			.navigationTitle("New post")
			.navigationBarTitleDisplayMode(.inline)
			.toolbar {
				ToolbarItem(placement: .cancellationAction) {
					Button("Cancel") { dismiss() }
						.disabled(posting)
				}
				ToolbarItem(placement: .confirmationAction) {
					if posting {
						ProgressView()
					} else {
						Button("Share", action: share)
							.fontWeight(.semibold)
							.disabled(image == nil || caption.count > captionLimit)
					}
				}
			}
			.photosPicker(isPresented: $pickerShown, selection: $pickedItem, matching: .images)
			.onChange(of: pickedItem) { _, item in
				Task { await load(item) }
			}
			.onAppear { if image == nil { pickerShown = true } }
			.interactiveDismissDisabled(posting)
		}
	}

	@ViewBuilder
	private var photoArea: some View {
		if let image {
			Color.clear
				.aspectRatio(4 / 5, contentMode: .fit)
				.overlay {
					Image(uiImage: image)
						.resizable()
						.scaledToFill()
				}
				.clipped()
				.overlay(alignment: .bottomTrailing) {
					Button { pickerShown = true } label: {
						Label("Change", systemImage: "photo.on.rectangle")
							.font(.footnote.weight(.semibold))
							.padding(.horizontal, 12)
							.padding(.vertical, 8)
							.background(.ultraThinMaterial, in: Capsule())
					}
					.buttonStyle(.plain)
					.padding(12)
				}
		} else {
			Button { pickerShown = true } label: {
				VStack(spacing: 12) {
					Image(systemName: "photo.on.rectangle.angled")
						.font(.system(size: 44, weight: .light))
					Text("Choose a photo")
						.font(.headline)
					Text("Pick one from your library to share.")
						.font(.subheadline)
						.foregroundStyle(.secondary)
				}
				.frame(maxWidth: .infinity)
				.aspectRatio(4 / 5, contentMode: .fit)
				.background(Theme.fieldFill)
			}
			.buttonStyle(.plain)
		}
	}

	private func load(_ item: PhotosPickerItem?) async {
		guard let item else { return }
		do {
			guard let data = try await item.loadTransferable(type: Data.self),
			      let picked = UIImage(data: data) else {
				errorMessage = MediaError.unreadableImage.errorDescription
				return
			}
			image = picked
			errorMessage = nil
			captionFocused = true
		} catch {
			errorMessage = MediaError.unreadableImage.errorDescription
		}
	}

	private func share() {
		guard let image, !posting else { return }
		guard let jpeg = image.jpegForUpload() else {
			errorMessage = MediaError.unreadableImage.errorDescription
			return
		}
		posting = true
		errorMessage = nil
		captionFocused = false
		Task {
			do {
				try await social.createPost(jpeg: jpeg, caption: caption.trimmingCharacters(in: .whitespacesAndNewlines))
				onPosted()
				dismiss()
			} catch {
				errorMessage = friendlyMessage(error)
				posting = false
			}
		}
	}
}
