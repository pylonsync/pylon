import SwiftUI

/// Likes and comments on the user's posts, and new followers. Live.
struct ActivityView: View {
	@EnvironmentObject private var social: SocialStore

	var body: some View {
		let items = social.activity
		ScrollView {
			if items.isEmpty {
				ContentUnavailableView(
					"No activity yet",
					systemImage: "heart",
					description: Text("When people like or comment on your posts, or follow you, it shows up here.")
				)
				.padding(.top, 80)
			} else {
				LazyVStack(alignment: .leading, spacing: 0, pinnedViews: []) {
					ForEach(sections(items), id: \.title) { section in
						Text(section.title)
							.font(.headline)
							.padding(.horizontal, 16)
							.padding(.top, 18)
							.padding(.bottom, 6)
						ForEach(section.items) { item in
							ActivityRow(item: item)
						}
					}
				}
				.animation(.snappy, value: items.map(\.id))
			}
		}
		.navigationTitle("Activity")
		.appRoutes()
	}

	private struct Section {
		let title: String
		let items: [ActivityItem]
	}

	private func sections(_ items: [ActivityItem]) -> [Section] {
		let calendar = Calendar.current
		let weekAgo = Date.now.addingTimeInterval(-7 * 86_400)
		let today = items.filter { calendar.isDateInToday($0.date) }
		let week = items.filter { !calendar.isDateInToday($0.date) && $0.date > weekAgo }
		let earlier = items.filter { $0.date <= weekAgo }
		return [
			Section(title: "Today", items: today),
			Section(title: "This week", items: week),
			Section(title: "Earlier", items: earlier),
		].filter { !$0.items.isEmpty }
	}
}

private struct ActivityRow: View {
	@EnvironmentObject private var social: SocialStore
	let item: ActivityItem

	private var actor: Profile? { social.profile(item.actorId) }

	var body: some View {
		NavigationLink(value: item.postId.map { Route.post($0) } ?? Route.profile(item.actorId)) {
			HStack(spacing: 12) {
				StoryAvatar(profile: actor, size: 44, ringed: social.hasRecentPost(item.actorId))
				sentence
					.font(.subheadline)
					.lineLimit(3)
					.frame(maxWidth: .infinity, alignment: .leading)
				trailing
			}
			.padding(.horizontal, 16)
			.padding(.vertical, 8)
			.contentShape(Rectangle())
		}
		.buttonStyle(.plain)
	}

	private var sentence: Text {
		let name = Text(actor?.handle ?? "Someone").fontWeight(.semibold)
		let time = Text(" " + Timestamp.short(item.date)).foregroundColor(.secondary)
		switch item.kind {
		case .like:
			return name + Text(" liked your photo.") + time
		case .comment:
			return name + Text(" commented: \(item.text ?? "")") + time
		case .follow:
			return name + Text(" started following you.") + time
		}
	}

	@ViewBuilder
	private var trailing: some View {
		if let postId = item.postId, let post = social.post(postId) {
			RemoteImage(url: social.media.thumbnailURL(for: post.imageUrl, width: 128))
				.frame(width: 44, height: 44)
				.clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
		} else if item.kind == .follow {
			let following = social.isFollowing(item.actorId)
			Button(following ? "Following" : "Follow back") {
				social.toggleFollow(item.actorId)
			}
			.buttonStyle(ProfileButtonStyle(prominent: !following))
			.frame(width: 112)
		}
	}
}
