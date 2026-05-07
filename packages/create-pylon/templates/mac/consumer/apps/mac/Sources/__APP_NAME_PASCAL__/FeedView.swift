import SwiftUI
import PylonClient
import PylonSync
import PylonSwiftUI

/// Live feed. Three subscriptions (Post / Like / Profile) joined
/// in-memory. Every new post + every like from any client renders
/// here without polling.
struct FeedView: View {
	@EnvironmentObject var session: AppSession
	let engine: SyncEngine
	let me: Profile
	let profiles: [Profile]

	@StateObject private var posts: PylonQuery<Post>
	@StateObject private var likes: PylonQuery<Like>
	@State private var draft = ""
	@State private var posting = false
	@State private var errorMessage: String?

	init(engine: SyncEngine, me: Profile, profiles: [Profile]) {
		self.engine = engine
		self.me = me
		self.profiles = profiles
		_posts = StateObject(
			wrappedValue: PylonQuery<Post>(engine: engine, entity: "Post"),
		)
		_likes = StateObject(
			wrappedValue: PylonQuery<Like>(engine: engine, entity: "Like"),
		)
	}

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
					if items.isEmpty {
						Text("No posts yet.")
							.foregroundStyle(.secondary)
					} else {
						ForEach(items, id: \.post.id) { item in
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
		}
	}

	private struct FeedRow: Hashable {
		let post: Post
		let author: Profile?
		let likeCount: Int
		let likedByMe: Bool
	}

	private var profilesById: [String: Profile] {
		Dictionary(uniqueKeysWithValues: profiles.map { ($0.id, $0) })
	}

	private var items: [FeedRow] {
		posts.rows
			.sorted { $0.createdAt > $1.createdAt }
			.prefix(100)
			.map { post in
				let postLikes = likes.rows.filter { $0.postId == post.id }
				return FeedRow(
					post: post,
					author: profilesById[post.authorId],
					likeCount: postLikes.count,
					likedByMe: postLikes.contains { $0.profileId == me.id },
				)
			}
	}

	@ViewBuilder
	private func row(_ item: FeedRow) -> some View {
		VStack(alignment: .leading, spacing: 6) {
			HStack(alignment: .firstTextBaseline) {
				Text(item.author?.displayName ?? "Unknown")
					.font(.subheadline.weight(.medium))
				Text("@\(item.author?.handle ?? "?")")
					.font(.system(.caption, design: .monospaced))
					.foregroundStyle(.secondary)
				Spacer()
				Text(item.post.createdAt)
					.font(.caption2)
					.foregroundStyle(.tertiary)
			}
			Text(item.post.body)
				.font(.body)
				.fixedSize(horizontal: false, vertical: true)
			HStack {
				Button {
					Task { await toggleLike(item.post.id) }
				} label: {
					HStack(spacing: 4) {
						Image(systemName: item.likedByMe ? "heart.fill" : "heart")
						Text("\(item.likeCount)")
					}
					.font(.caption)
					.foregroundStyle(item.likedByMe ? .pink : .secondary)
				}
				.buttonStyle(.plain)

				if item.author?.id == me.id {
					Spacer()
					Button("Delete", role: .destructive) {
						Task { await delete(item.post.id) }
					}
					.font(.caption)
				}
			}
		}
		.padding(.vertical, 4)
	}

	// MARK: - Mutations (writes flow through callFn; the engine receives
	// the change_event and updates the local store, which re-renders the
	// PylonQuery rows above).

	private func post() async {
		let body = draft.trimmingCharacters(in: .whitespaces)
		guard !body.isEmpty else { return }
		posting = true
		defer { posting = false }
		do {
			// `createPost` returns the joined shape (Post + author),
			// but we don't use the return value — the engine will
			// surface the new Post row via the live subscription.
			let _: PostCreatedResponse = try await session.client.callFn(
				"createPost",
				args: CreatePostArgs(body: body),
			)
			draft = ""
			errorMessage = nil
		} catch {
			errorMessage = "Post failed: \(error.localizedDescription)"
		}
	}

	private func toggleLike(_ postId: String) async {
		do {
			let _: ToggleLikeResponse = try await session.client.callFn(
				"toggleLike",
				args: ToggleLikeArgs(postId: postId),
			)
		} catch {
			errorMessage = "Like failed: \(error.localizedDescription)"
		}
	}

	private func delete(_ postId: String) async {
		do {
			let _: Post = try await session.client.callFn(
				"deletePost",
				args: DeletePostArgs(id: postId),
			)
		} catch {
			errorMessage = "Delete failed: \(error.localizedDescription)"
		}
	}
}

private struct PostCreatedResponse: Decodable {
	let id: String
}

private struct ToggleLikeResponse: Decodable {
	let liked: Bool
	let likeCount: Int
}
