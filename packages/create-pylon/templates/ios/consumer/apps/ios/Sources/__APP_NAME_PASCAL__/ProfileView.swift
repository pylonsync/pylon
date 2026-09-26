import SwiftUI

/// A profile: photo, counts, bio, follow or edit, and a three-column grid of posts.
struct ProfileView: View {
	@EnvironmentObject private var app: AppModel
	@EnvironmentObject private var social: SocialStore
	let profileId: String
	/// True on the Profile tab, where the toolbar holds sign-out.
	let isRoot: Bool

	@State private var editing = false
	@State private var confirmSignOut = false

	private var profile: Profile? { social.profile(profileId) }
	private var isMe: Bool { profileId == social.myProfileId }
	private var posts: [Post] { social.posts(by: profileId) }

	var body: some View {
		ScrollView {
			VStack(alignment: .leading, spacing: 0) {
				header
				grid
			}
		}
		.navigationTitle(profile?.handle ?? "")
		.navigationBarTitleDisplayMode(.inline)
		.toolbar {
			if isRoot {
				ToolbarItem(placement: .topBarTrailing) {
					Menu {
						Button("Edit profile", systemImage: "pencil") { editing = true }
						Button("Sign out", systemImage: "rectangle.portrait.and.arrow.right", role: .destructive) {
							confirmSignOut = true
						}
					} label: {
						Image(systemName: "line.3.horizontal")
					}
					.accessibilityLabel("Settings")
				}
			}
		}
		.sheet(isPresented: $editing) {
			NavigationStack { ProfileEditorView(mode: .edit) }
				.environmentObject(social)
		}
		.confirmationDialog("Sign out of __APP_NAME__?", isPresented: $confirmSignOut, titleVisibility: .visible) {
			Button("Sign out", role: .destructive) { Task { await app.signOut() } }
		}
		.modifier(RootRoutes(isRoot: isRoot))
	}

	private var header: some View {
		VStack(alignment: .leading, spacing: 12) {
			HStack(spacing: 20) {
				StoryAvatar(profile: profile, size: 84, ringed: social.hasRecentPost(profileId))
				HStack(spacing: 0) {
					stat(posts.count, "posts")
					stat(social.followerCount(profileId), social.followerCount(profileId) == 1 ? "follower" : "followers")
					stat(social.followingIds(of: profileId).count, "following")
				}
			}
			VStack(alignment: .leading, spacing: 2) {
				Text(profile?.displayName ?? "")
					.font(.subheadline.weight(.semibold))
				if let bio = profile?.bio, !bio.isEmpty {
					Text(bio)
						.font(.subheadline)
				}
			}
			actionButtons
		}
		.padding(.horizontal, 16)
		.padding(.top, 8)
		.padding(.bottom, 14)
	}

	private func stat(_ value: Int, _ label: String) -> some View {
		VStack(spacing: 2) {
			Text(value.compactCount)
				.font(.headline)
				.contentTransition(.numericText())
			Text(label)
				.font(.footnote)
				.foregroundStyle(.secondary)
		}
		.frame(maxWidth: .infinity)
		.animation(.snappy, value: value)
	}

	@ViewBuilder
	private var actionButtons: some View {
		HStack(spacing: 6) {
			if isMe {
				Button("Edit profile") { editing = true }
					.buttonStyle(ProfileButtonStyle())
				if let profile {
					ShareLink(item: "@\(profile.handle) on __APP_NAME__") {
						Text("Share profile")
					}
					.buttonStyle(ProfileButtonStyle())
				}
			} else {
				let following = social.isFollowing(profileId)
				Button(following ? "Following" : "Follow") {
					social.toggleFollow(profileId)
				}
				.buttonStyle(ProfileButtonStyle(prominent: !following))
				.sensoryFeedback(.selection, trigger: following)
			}
		}
	}

	@ViewBuilder
	private var grid: some View {
		Divider()
		if posts.isEmpty {
			VStack(spacing: 10) {
				Image(systemName: "camera")
					.font(.system(size: 34, weight: .light))
					.frame(width: 72, height: 72)
					.overlay(Circle().stroke(Color.primary, lineWidth: 1.5))
				Text(isMe ? "Share your first photo" : "No posts yet")
					.font(.title3.weight(.semibold))
				if isMe {
					Text("Photos you share show up on your profile.")
						.font(.subheadline)
						.foregroundStyle(.secondary)
				}
			}
			.frame(maxWidth: .infinity)
			.padding(.top, 48)
		} else {
			LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 2), count: 3), spacing: 2) {
				ForEach(posts) { post in
					NavigationLink(value: Route.profilePosts(profileId: profileId, startAt: post.id)) {
						Color.clear
							.aspectRatio(1, contentMode: .fit)
							.overlay {
								RemoteImage(url: social.media.thumbnailURL(for: post.imageUrl, width: 384))
							}
							.clipped()
					}
					.buttonStyle(.plain)
					.accessibilityLabel(post.caption ?? "Photo")
				}
			}
			.padding(.top, 2)
			.animation(.snappy, value: posts.map(\.id))
		}
	}
}

/// Registers route destinations once, on the Profile tab's root view.
/// Pushed profiles share their stack's registration.
private struct RootRoutes: ViewModifier {
	let isRoot: Bool

	func body(content: Content) -> some View {
		if isRoot { content.appRoutes() } else { content }
	}
}

/// One person's posts as a feed, opened at the post tapped in their grid.
struct ProfilePostsView: View {
	@EnvironmentObject private var social: SocialStore
	let profileId: String
	let startAt: String

	var body: some View {
		ScrollViewReader { proxy in
			ScrollView {
				LazyVStack(spacing: 0) {
					ForEach(social.posts(by: profileId)) { post in
						PostCard(post: post).id(post.id)
					}
				}
			}
			.onAppear { proxy.scrollTo(startAt, anchor: .top) }
		}
		.navigationTitle("Posts")
		.navigationBarTitleDisplayMode(.inline)
	}
}
