import SwiftUI

/// The home feed: stories row, then every post, newest first. New posts
/// from other people arrive over sync and slide in at the top.
struct FeedView: View {
	@EnvironmentObject private var social: SocialStore
	/// Changes when the feed should scroll back to the top (after posting).
	let scrollToken: Int
	let onCompose: () -> Void

	@State private var storyStart: StoryStart?
	@State private var newPostAuthor: Profile?
	@State private var knownTopPostId: String?
	@State private var atTop = true
	@State private var pillTimer: Task<Void, Never>?

	var body: some View {
		ScrollViewReader { proxy in
			ScrollView {
				LazyVStack(spacing: 0) {
					Color.clear.frame(height: 0).id("top")
						.onAppear { atTop = true }
						.onDisappear { atTop = false }
					StoriesRow(onCompose: onCompose) { authors, index in
						storyStart = StoryStart(authors: authors, index: index)
					}
					Divider().opacity(0.6)

					if social.isLoading && social.posts.isEmpty {
						ForEach(0..<2, id: \.self) { _ in PostSkeleton() }
					} else if social.posts.isEmpty {
						ContentUnavailableView(
							"No posts yet",
							systemImage: "photo.on.rectangle",
							description: Text("Photos you and other people share show up here.")
						)
						.padding(.top, 60)
					} else {
						ForEach(social.posts) { post in
							PostCard(post: post)
								.transition(.asymmetric(insertion: .move(edge: .top).combined(with: .opacity), removal: .opacity))
						}
					}
				}
				.animation(.spring(response: 0.45, dampingFraction: 0.85), value: social.posts.map(\.id))
			}
			.refreshable { await social.engine.pull() }
			.safeAreaInset(edge: .top, spacing: 0) { header }
			.overlay(alignment: .top) {
				if let author = newPostAuthor {
					NewPostPill(author: author) {
						withAnimation { proxy.scrollTo("top", anchor: .top) }
						newPostAuthor = nil
					}
					.padding(.top, 64)
					.transition(.move(edge: .top).combined(with: .opacity))
				}
			}
			.onChange(of: scrollToken) { _, _ in
				withAnimation { proxy.scrollTo("top", anchor: .top) }
			}
			.onChange(of: social.posts.first?.id) { _, newTop in
				announceNewPost(topId: newTop)
			}
		}
		.toolbar(.hidden, for: .navigationBar)
		.appRoutes()
		.fullScreenCover(item: $storyStart) { start in
			StoryViewer(authors: start.authors, startIndex: start.index)
				.environmentObject(social)
		}
	}

	private var header: some View {
		HStack {
			Wordmark(size: 30)
			Spacer()
			Button(action: onCompose) {
				Image(systemName: "plus.app")
					.font(.system(size: 24))
			}
			.accessibilityLabel("New post")
			.foregroundStyle(.primary)
		}
		.padding(.horizontal, 16)
		.frame(height: 50)
		.background(.bar)
	}

	/// Shows "New post from @handle" when someone else's post lands on top
	/// while the user is scrolled down the feed.
	private func announceNewPost(topId: String?) {
		defer { knownTopPostId = topId }
		guard let topId, let known = knownTopPostId, topId != known,
		      let post = social.post(topId), post.authorId != social.myProfileId,
		      post.date > .now.addingTimeInterval(-120) else { return }
		guard !atTop else { return }
		withAnimation(.spring(response: 0.4, dampingFraction: 0.8)) {
			newPostAuthor = social.profile(post.authorId)
		}
		pillTimer?.cancel()
		pillTimer = Task {
			try? await Task.sleep(for: .seconds(5))
			guard !Task.isCancelled else { return }
			withAnimation { newPostAuthor = nil }
		}
	}
}

struct StoryStart: Identifiable {
	let id = UUID()
	let authors: [Profile]
	let index: Int
}

/// The row of avatars at the top of the feed. A ring means the person
/// posted in the last 24 hours. Tapping opens those posts full screen.
struct StoriesRow: View {
	@EnvironmentObject private var social: SocialStore
	let onCompose: () -> Void
	let onOpen: ([Profile], Int) -> Void

	var body: some View {
		let authors = social.storyAuthors
		ScrollView(.horizontal, showsIndicators: false) {
			HStack(alignment: .top, spacing: 14) {
				if let me = social.me {
					Button {
						if social.hasRecentPost(me.id) { onOpen([me], 0) } else { onCompose() }
					} label: {
						StoryBubble(profile: me, label: "Your story", ringed: social.hasRecentPost(me.id), showsAdd: true)
					}
					.buttonStyle(.plain)
				}
				ForEach(Array(authors.enumerated()), id: \.element.id) { index, author in
					Button { onOpen(authors, index) } label: {
						StoryBubble(profile: author, label: author.handle, ringed: true, showsAdd: false)
					}
					.buttonStyle(.plain)
				}
			}
			.padding(.horizontal, 12)
			.padding(.vertical, 10)
		}
	}
}

private struct StoryBubble: View {
	let profile: Profile
	let label: String
	let ringed: Bool
	let showsAdd: Bool

	var body: some View {
		VStack(spacing: 5) {
			StoryAvatar(profile: profile, size: 64, ringed: ringed)
				.overlay(alignment: .bottomTrailing) {
					if showsAdd {
						Image(systemName: "plus")
							.font(.system(size: 11, weight: .heavy))
							.foregroundStyle(.white)
							.frame(width: 22, height: 22)
							.background(Theme.accent, in: Circle())
							.overlay(Circle().stroke(Color(uiColor: .systemBackground), lineWidth: 2.5))
							.offset(x: -2, y: -2)
					}
				}
			Text(label)
				.font(.caption)
				.foregroundStyle(showsAdd ? .secondary : .primary)
				.lineLimit(1)
				.frame(width: 74)
		}
	}
}

private struct NewPostPill: View {
	let author: Profile
	let action: () -> Void

	var body: some View {
		Button(action: action) {
			HStack(spacing: 8) {
				AvatarView(profile: author, size: 22)
				Text("New post from \(author.handle)")
					.font(.subheadline.weight(.semibold))
				Image(systemName: "arrow.up")
					.font(.footnote.weight(.bold))
			}
			.foregroundStyle(.white)
			.padding(.leading, 6)
			.padding(.trailing, 14)
			.padding(.vertical, 6)
			.background(Theme.accent, in: Capsule())
			.shadow(color: .black.opacity(0.18), radius: 10, y: 4)
		}
		.buttonStyle(.plain)
	}
}

private struct PostSkeleton: View {
	var body: some View {
		VStack(alignment: .leading, spacing: 10) {
			HStack(spacing: 10) {
				Circle().frame(width: 34, height: 34)
				RoundedRectangle(cornerRadius: 4).frame(width: 120, height: 12)
			}
			.padding(.horizontal, 12)
			Rectangle().aspectRatio(4 / 5, contentMode: .fit)
			RoundedRectangle(cornerRadius: 4).frame(width: 180, height: 12).padding(.horizontal, 14)
		}
		.foregroundStyle(Theme.fieldFill)
		.padding(.vertical, 8)
	}
}
