import SwiftUI
import PylonClient

struct FeedView: View {
	@EnvironmentObject var session: AppSession
	@State private var feed: [FeedItem] = []
	@State private var loading = true
	@State private var posting = false
	@State private var draft = ""
	@State private var errorMessage: String?

	var body: some View {
		NavigationStack {
			List {
				Section("Post") {
					VStack(alignment: .leading, spacing: 8) {
						TextEditor(text: $draft)
							.frame(minHeight: 80)
							.font(.body)
						HStack {
							Text("\(draft.count)/1000")
								.font(.caption)
								.foregroundStyle(.secondary)
							Spacer()
							Button(posting ? "Posting…" : "Post") {
								Task { await post() }
							}
							.disabled(posting || draft.trimmingCharacters(in: .whitespaces).isEmpty)
						}
					}
				}

				Section("Feed") {
					if loading {
						ProgressView()
					} else if feed.isEmpty {
						Text("No posts yet.")
							.foregroundStyle(.secondary)
					} else {
						ForEach(feed) { item in
							row(item)
						}
					}
				}

				if let errorMessage {
					Section {
						Text(errorMessage)
							.foregroundStyle(.red)
							.font(.caption)
					}
				}
			}
			.navigationTitle("__APP_NAME__")
			.task { await load() }
			.refreshable { await load() }
		}
	}

	@ViewBuilder
	private func row(_ item: FeedItem) -> some View {
		VStack(alignment: .leading, spacing: 6) {
			HStack(alignment: .firstTextBaseline) {
				Text(item.author?.displayName ?? "Unknown")
					.font(.subheadline.weight(.medium))
				Text("@\(item.author?.handle ?? "?")")
					.font(.system(.caption, design: .monospaced))
					.foregroundStyle(.secondary)
				Spacer()
				Text(item.createdAt)
					.font(.caption2)
					.foregroundStyle(.tertiary)
			}
			Text(item.body)
				.font(.body)
				.fixedSize(horizontal: false, vertical: true)
			HStack {
				Button {
					Task { await toggleLike(item) }
				} label: {
					HStack(spacing: 4) {
						Image(systemName: item.likedByMe ? "heart.fill" : "heart")
						Text("\(item.likeCount)")
					}
					.font(.caption)
					.foregroundStyle(item.likedByMe ? .pink : .secondary)
				}
				.buttonStyle(.plain)

				if item.author?.id == session.me?.id {
					Spacer()
					Button("Delete", role: .destructive) {
						Task { await delete(item) }
					}
					.font(.caption)
				}
			}
		}
		.padding(.vertical, 4)
	}

	// MARK: - Network

	private func load() async {
		loading = true
		defer { loading = false }
		do {
			feed = try await session.pylon.callFn("feed", args: EmptyArgs())
			errorMessage = nil
		} catch {
			errorMessage = "Load failed: \(error.localizedDescription)"
		}
	}

	private func post() async {
		let body = draft.trimmingCharacters(in: .whitespaces)
		guard !body.isEmpty else { return }
		posting = true
		defer { posting = false }
		do {
			let item: FeedItem = try await session.pylon.callFn(
				"createPost",
				args: CreatePostArgs(body: body),
			)
			feed.insert(item, at: 0)
			draft = ""
		} catch {
			errorMessage = "Post failed: \(error.localizedDescription)"
		}
	}

	private func toggleLike(_ item: FeedItem) async {
		// Optimistic
		if let i = feed.firstIndex(where: { $0.id == item.id }) {
			feed[i].likedByMe.toggle()
			feed[i].likeCount += feed[i].likedByMe ? 1 : -1
		}
		do {
			let result: ToggleLikeResult = try await session.pylon.callFn(
				"toggleLike",
				args: ToggleLikeArgs(postId: item.id),
			)
			if let i = feed.firstIndex(where: { $0.id == item.id }) {
				feed[i].likedByMe = result.liked
				feed[i].likeCount = result.likeCount
			}
		} catch {
			// Revert
			if let i = feed.firstIndex(where: { $0.id == item.id }) {
				feed[i].likedByMe = item.likedByMe
				feed[i].likeCount = item.likeCount
			}
			errorMessage = "Like failed: \(error.localizedDescription)"
		}
	}

	private func delete(_ item: FeedItem) async {
		let snapshot = feed
		feed.removeAll { $0.id == item.id }
		do {
			let _: FeedItem = try await session.pylon.callFn(
				"deletePost",
				args: DeletePostArgs(id: item.id),
			)
		} catch {
			feed = snapshot
			errorMessage = "Delete failed: \(error.localizedDescription)"
		}
	}
}
