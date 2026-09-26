import SwiftUI

/// A mosaic of every post, plus people search by name or username.
struct ExploreView: View {
	@EnvironmentObject private var social: SocialStore
	@State private var query = ""

	var body: some View {
		ScrollView {
			if query.trimmingCharacters(in: .whitespaces).isEmpty {
				ExploreMosaic(posts: social.posts)
			} else {
				peopleResults
			}
		}
		.navigationTitle("Explore")
		.searchable(text: $query, placement: .navigationBarDrawer(displayMode: .always), prompt: "Search people")
		.textInputAutocapitalization(.never)
		.autocorrectionDisabled()
		.appRoutes()
	}

	private var matches: [Profile] {
		let q = query.trimmingCharacters(in: .whitespaces).lowercased()
		return social.profiles.values
			.filter { $0.handle.contains(q) || $0.displayName.lowercased().contains(q) }
			.sorted { social.followerCount($0.id) > social.followerCount($1.id) }
	}

	@ViewBuilder
	private var peopleResults: some View {
		if matches.isEmpty {
			ContentUnavailableView.search(text: query)
				.padding(.top, 40)
		} else {
			LazyVStack(spacing: 0) {
				ForEach(matches) { profile in
					NavigationLink(value: Route.profile(profile.id)) {
						HStack(spacing: 12) {
							StoryAvatar(profile: profile, size: 48, ringed: social.hasRecentPost(profile.id))
							VStack(alignment: .leading, spacing: 2) {
								Text(profile.handle).font(.subheadline.weight(.semibold))
								Text("\(profile.displayName) · \(social.followerCount(profile.id).compactCount) followers")
									.font(.subheadline)
									.foregroundStyle(.secondary)
							}
							Spacer()
						}
						.padding(.horizontal, 16)
						.padding(.vertical, 8)
						.contentShape(Rectangle())
					}
					.buttonStyle(.plain)
				}
			}
			.padding(.top, 8)
		}
	}
}

/// Posts in repeating blocks of six: one large tile beside two small ones
/// (the large tile alternates sides), then a row of three.
private struct ExploreMosaic: View {
	@EnvironmentObject private var social: SocialStore
	let posts: [Post]
	@State private var width: CGFloat = 0
	private let gap: CGFloat = 2

	private var small: CGFloat { max(0, (width - gap * 2) / 3) }
	private var large: CGFloat { small * 2 + gap }

	var body: some View {
		let blocks = stride(from: 0, to: posts.count, by: 6).map { Array(posts[$0..<min($0 + 6, posts.count)]) }
		LazyVStack(alignment: .leading, spacing: gap) {
			if width > 0 {
				ForEach(Array(blocks.enumerated()), id: \.offset) { index, block in
					blockView(block, largeOnLeft: index % 2 == 0)
				}
			}
		}
		.frame(maxWidth: .infinity, alignment: .leading)
		.onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
	}

	@ViewBuilder
	private func blockView(_ block: [Post], largeOnLeft: Bool) -> some View {
		if block.count >= 3 {
			HStack(spacing: gap) {
				if largeOnLeft { tile(block[0], size: large) }
				VStack(spacing: gap) {
					tile(block[1], size: small)
					tile(block[2], size: small)
				}
				if !largeOnLeft { tile(block[0], size: large) }
			}
			row(Array(block.dropFirst(3)))
		} else {
			row(block)
		}
	}

	private func row(_ items: [Post]) -> some View {
		HStack(spacing: gap) {
			ForEach(items) { tile($0, size: small) }
		}
	}

	private func tile(_ post: Post, size: CGFloat) -> some View {
		NavigationLink(value: Route.post(post.id)) {
			RemoteImage(url: social.media.thumbnailURL(for: post.imageUrl, width: size > 200 ? 828 : 384))
				.frame(width: size, height: size)
				.clipped()
		}
		.buttonStyle(.plain)
		.accessibilityLabel(post.caption ?? "Photo")
	}
}
