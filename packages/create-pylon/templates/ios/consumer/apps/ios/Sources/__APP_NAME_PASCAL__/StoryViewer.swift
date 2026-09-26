import SwiftUI

/// Full-screen playback of each person's posts from the last 24 hours.
/// Every photo shows for five seconds. Tap the right side for the next
/// photo, the left side for the previous one, or swipe down to close.
struct StoryViewer: View {
	@EnvironmentObject private var social: SocialStore
	@Environment(\.dismiss) private var dismiss

	let authors: [Profile]
	@State var authorIndex: Int
	@State private var postIndex = 0
	@State private var progress: Double = 0
	@State private var dragOffset: CGFloat = 0
	/// Bumped to replay the current photo from the start.
	@State private var restarts = 0

	private let duration: Double = 5
	private let tick: Double = 0.05

	init(authors: [Profile], startIndex: Int) {
		self.authors = authors
		_authorIndex = State(initialValue: startIndex)
	}

	private var author: Profile? { authors.indices.contains(authorIndex) ? authors[authorIndex] : nil }
	private var posts: [Post] { author.map { social.recentPosts(by: $0.id) } ?? [] }
	private var post: Post? { posts.indices.contains(postIndex) ? posts[postIndex] : nil }

	var body: some View {
		ZStack {
			Color.black.ignoresSafeArea()
			if let post {
				let url = social.media.url(for: post.imageUrl)
				RemoteImage(url: url)
					.blur(radius: 40)
					.opacity(0.6)
					.ignoresSafeArea()
				Color.clear
					.aspectRatio(4 / 5, contentMode: .fit)
					.overlay { RemoteImage(url: url) }
					.clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
					.id(post.id)
			}
			HStack(spacing: 0) {
				Color.clear.contentShape(Rectangle()).onTapGesture(perform: previous)
				Color.clear.contentShape(Rectangle()).onTapGesture(perform: next)
			}
			VStack(spacing: 10) {
				progressBars
				topBar
				Spacer()
				if let caption = post?.caption, !caption.isEmpty {
					Text(caption)
						.font(.subheadline)
						.foregroundStyle(.white)
						.multilineTextAlignment(.leading)
						.frame(maxWidth: .infinity, alignment: .leading)
						.padding(14)
						.background(.black.opacity(0.35), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
				}
			}
			.padding(.horizontal, 12)
			.padding(.top, 8)
		}
		.offset(y: dragOffset)
		.gesture(
			DragGesture()
				.onChanged { value in dragOffset = max(0, value.translation.height) }
				.onEnded { value in
					if value.translation.height > 120 { dismiss() } else {
						withAnimation(.spring) { dragOffset = 0 }
					}
				}
		)
		.statusBarHidden()
		.task(id: "\(authorIndex)-\(postIndex)-\(restarts)") { await play() }
	}

	private var progressBars: some View {
		HStack(spacing: 4) {
			ForEach(posts.indices, id: \.self) { index in
				GeometryReader { geo in
					Capsule().fill(.white.opacity(0.35))
						.overlay(alignment: .leading) {
							Capsule().fill(.white)
								.frame(width: geo.size.width * fill(for: index))
						}
				}
				.frame(height: 2.5)
			}
		}
	}

	private var topBar: some View {
		HStack(spacing: 10) {
			AvatarView(profile: author, size: 32)
			Text(author?.handle ?? "")
				.font(.subheadline.weight(.semibold))
			if let post {
				Text(Timestamp.short(post.date))
					.font(.subheadline)
					.opacity(0.7)
			}
			Spacer()
			Button { dismiss() } label: {
				Image(systemName: "xmark")
					.font(.system(size: 20, weight: .semibold))
					.frame(width: 40, height: 40)
					.contentShape(Rectangle())
			}
			.accessibilityLabel("Close")
		}
		.foregroundStyle(.white)
	}

	private func fill(for index: Int) -> Double {
		if index < postIndex { return 1 }
		if index == postIndex { return progress }
		return 0
	}

	private func play() async {
		progress = 0
		let steps = Int(duration / tick)
		for _ in 0..<steps {
			try? await Task.sleep(for: .seconds(tick))
			if Task.isCancelled { return }
			progress += tick / duration
		}
		next()
	}

	private func next() {
		if postIndex + 1 < posts.count {
			postIndex += 1
		} else if authorIndex + 1 < authors.count {
			authorIndex += 1
			postIndex = 0
		} else {
			dismiss()
		}
	}

	private func previous() {
		if postIndex > 0 {
			postIndex -= 1
		} else if authorIndex > 0 {
			authorIndex -= 1
			postIndex = 0
		} else {
			restarts += 1
		}
	}
}
