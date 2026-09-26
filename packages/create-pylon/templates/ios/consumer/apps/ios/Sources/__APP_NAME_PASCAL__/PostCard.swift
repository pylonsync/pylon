import SwiftUI

/// Pushed screens, shared by every tab's NavigationStack.
enum Route: Hashable {
	case profile(String)
	case post(String)
	case profilePosts(profileId: String, startAt: String)
}

extension View {
	/// Registers the destinations for `Route` values on a NavigationStack.
	func appRoutes() -> some View {
		navigationDestination(for: Route.self) { route in
			switch route {
			case .profile(let id): ProfileView(profileId: id, isRoot: false)
			case .post(let id): PostDetailView(postId: id)
			case .profilePosts(let profileId, let startAt): ProfilePostsView(profileId: profileId, startAt: startAt)
			}
		}
	}
}

/// One post in a feed: author, photo, actions, likes, caption, comments.
struct PostCard: View {
	@EnvironmentObject private var social: SocialStore
	let post: Post

	@State private var showBurst = false
	@State private var burstTrigger = 0
	@State private var showComments = false
	@State private var confirmDelete = false

	private var author: Profile? { social.profile(post.authorId) }
	private var liked: Bool { social.isLiked(post.id) }
	private var likeCount: Int { social.likeCount(post.id) }
	private var comments: [Comment] { social.comments(post.id) }

	var body: some View {
		VStack(alignment: .leading, spacing: 0) {
			header
			photo
			actions
			details
		}
		.padding(.bottom, 14)
		.sheet(isPresented: $showComments) {
			CommentsView(postId: post.id)
				.environmentObject(social)
				.presentationDetents([.medium, .large])
				.presentationDragIndicator(.visible)
		}
		.confirmationDialog("Delete this post?", isPresented: $confirmDelete, titleVisibility: .visible) {
			Button("Delete", role: .destructive) {
				Task { try? await social.deletePost(post.id) }
			}
		} message: {
			Text("The photo, its likes, and its comments are removed for everyone.")
		}
	}

	private var header: some View {
		HStack(spacing: 10) {
			NavigationLink(value: Route.profile(post.authorId)) {
				HStack(spacing: 10) {
					StoryAvatar(profile: author, size: 34, ringed: social.hasRecentPost(post.authorId))
					HStack(spacing: 5) {
						Text(author?.handle ?? "")
							.font(.subheadline.weight(.semibold))
						Text(Timestamp.short(post.date))
							.font(.subheadline)
							.foregroundStyle(.secondary)
					}
				}
			}
			.buttonStyle(.plain)
			Spacer()
			Menu {
				if let url = social.media.url(for: post.imageUrl) {
					ShareLink(item: url) { Label("Share", systemImage: "square.and.arrow.up") }
				}
				if post.authorId == social.myProfileId {
					Button(role: .destructive) { confirmDelete = true } label: {
						Label("Delete", systemImage: "trash")
					}
				}
			} label: {
				Image(systemName: "ellipsis")
					.font(.system(size: 16, weight: .semibold))
					.frame(width: 36, height: 36)
					.contentShape(Rectangle())
			}
			.foregroundStyle(.primary)
			.accessibilityLabel("More")
		}
		.padding(.leading, 12)
		.padding(.trailing, 4)
		.padding(.vertical, 8)
	}

	private var photo: some View {
		Color.clear
			.aspectRatio(4 / 5, contentMode: .fit)
			.overlay { RemoteImage(url: social.media.url(for: post.imageUrl)) }
			.clipped()
			.overlay {
				Image(systemName: "heart.fill")
					.font(.system(size: 100))
					.foregroundStyle(.white)
					.shadow(color: .black.opacity(0.25), radius: 12, y: 4)
					.scaleEffect(showBurst ? 1 : 0.3)
					.opacity(showBurst ? 1 : 0)
					.allowsHitTesting(false)
			}
			.contentShape(Rectangle())
			.onTapGesture(count: 2) {
				social.like(post.id)
				burstTrigger += 1
				withAnimation(.spring(response: 0.28, dampingFraction: 0.5)) { showBurst = true }
				Task {
					try? await Task.sleep(for: .milliseconds(650))
					withAnimation(.easeIn(duration: 0.2)) { showBurst = false }
				}
			}
			.sensoryFeedback(.impact(weight: .light), trigger: burstTrigger)
			.accessibilityLabel(post.caption ?? "Photo")
			.accessibilityAddTraits(.isImage)
	}

	private var actions: some View {
		HStack(spacing: 18) {
			Button {
				social.toggleLike(post.id)
			} label: {
				Image(systemName: liked ? "heart.fill" : "heart")
					.foregroundStyle(liked ? Theme.heart : .primary)
					.symbolEffect(.bounce, value: liked)
			}
			.accessibilityLabel(liked ? "Unlike" : "Like")
			.sensoryFeedback(.selection, trigger: liked)

			Button { showComments = true } label: {
				Image(systemName: "bubble.right")
			}
			.accessibilityLabel("Comment")

			if let url = social.media.url(for: post.imageUrl) {
				ShareLink(item: url) {
					Image(systemName: "paperplane")
				}
				.accessibilityLabel("Share")
			}
			Spacer()
		}
		.font(.system(size: 23))
		.foregroundStyle(.primary)
		.buttonStyle(.plain)
		.padding(.horizontal, 14)
		.padding(.top, 12)
		.padding(.bottom, 8)
	}

	private var details: some View {
		VStack(alignment: .leading, spacing: 5) {
			likesLine
			if let caption = post.caption, !caption.isEmpty {
				(Text(author?.handle ?? "").fontWeight(.semibold) + Text(" ") + Text(caption))
					.font(.subheadline)
					.lineLimit(3)
			}
			if !comments.isEmpty {
				Button {
					showComments = true
				} label: {
					Text(comments.count == 1 ? "View 1 comment" : "View all \(comments.count) comments")
						.font(.subheadline)
						.foregroundStyle(.secondary)
				}
				.buttonStyle(.plain)
			}
		}
		.padding(.horizontal, 14)
	}

	@ViewBuilder
	private var likesLine: some View {
		if likeCount > 0 {
			if let liker = social.featuredLiker(post.id), likeCount > 1 {
				HStack(spacing: 6) {
					AvatarView(profile: liker, size: 18)
					(Text("Liked by ") + Text(liker.handle).fontWeight(.semibold)
						+ Text(" and ") + Text(likeCount - 1 == 1 ? "1 other" : "\((likeCount - 1).compactCount) others").fontWeight(.semibold))
						.font(.subheadline)
						.lineLimit(1)
				}
				.contentTransition(.numericText())
			} else {
				Text(likeCount == 1 ? "1 like" : "\(likeCount.compactCount) likes")
					.font(.subheadline.weight(.semibold))
					.contentTransition(.numericText())
			}
		}
	}
}

/// An avatar with the gradient ring for someone who posted in the last 24 hours.
struct StoryAvatar: View {
	let profile: Profile?
	var size: CGFloat
	var ringed: Bool

	var body: some View {
		let ring = max(2, size * 0.045)
		let gap = max(2, size * 0.05)
		AvatarView(profile: profile, size: size)
			.padding(gap)
			.background(Circle().fill(Color(uiColor: .systemBackground)))
			.padding(ring)
			.background {
				if ringed {
					Circle().fill(Theme.storyRing)
				} else {
					Circle().stroke(Theme.hairline, lineWidth: 0.5).padding(ring)
				}
			}
	}
}

/// A single post on its own screen, reached from grids and activity.
struct PostDetailView: View {
	@EnvironmentObject private var social: SocialStore
	let postId: String

	var body: some View {
		ScrollView {
			if let post = social.post(postId) {
				PostCard(post: post)
			} else {
				ContentUnavailableView("Post removed", systemImage: "photo", description: Text("This post is no longer available."))
					.padding(.top, 80)
			}
		}
		.navigationTitle("Post")
		.navigationBarTitleDisplayMode(.inline)
	}
}
