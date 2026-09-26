import SwiftUI

/// The comments on one post. New comments from anyone appear live.
struct CommentsView: View {
	@EnvironmentObject private var social: SocialStore
	let postId: String

	@State private var draft = ""
	@State private var sending = false
	@State private var errorMessage: String?
	@FocusState private var inputFocused: Bool

	private var post: Post? { social.post(postId) }
	private var comments: [Comment] { social.comments(postId) }

	var body: some View {
		NavigationStack {
			ScrollViewReader { proxy in
				ScrollView {
					LazyVStack(alignment: .leading, spacing: 18) {
						if let post, let caption = post.caption, !caption.isEmpty {
							CommentRow(profile: social.profile(post.authorId), text: caption, date: post.date)
							Divider()
						}
						if comments.isEmpty {
							Text("No comments yet. Start the conversation.")
								.font(.subheadline)
								.foregroundStyle(.secondary)
								.frame(maxWidth: .infinity)
								.padding(.top, 24)
						}
						ForEach(comments) { comment in
							CommentRow(profile: social.profile(comment.profileId), text: comment.text, date: comment.date)
								.id(comment.id)
								.transition(.opacity.combined(with: .move(edge: .bottom)))
						}
					}
					.padding(16)
					.animation(.snappy, value: comments.map(\.id))
				}
				.onChange(of: comments.last?.id) { _, id in
					guard let id else { return }
					withAnimation { proxy.scrollTo(id, anchor: .bottom) }
				}
			}
			.safeAreaInset(edge: .bottom) { inputBar }
			.navigationTitle("Comments")
			.navigationBarTitleDisplayMode(.inline)
			.appRoutes()
		}
	}

	private var inputBar: some View {
		VStack(spacing: 6) {
			if let errorMessage {
				Text(errorMessage)
					.font(.footnote)
					.foregroundStyle(.red)
					.frame(maxWidth: .infinity, alignment: .leading)
			}
			HStack(spacing: 10) {
				AvatarView(profile: social.me, size: 34)
				HStack {
					TextField(placeholder, text: $draft, axis: .vertical)
						.lineLimit(1...4)
						.focused($inputFocused)
						.submitLabel(.send)
						.onSubmit(send)
					if !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
						Button(action: send) {
							if sending { ProgressView() } else { Text("Post").fontWeight(.semibold) }
						}
						.disabled(sending)
					}
				}
				.padding(.horizontal, 14)
				.padding(.vertical, 10)
				.background(Capsule().stroke(Theme.hairline))
			}
		}
		.padding(.horizontal, 12)
		.padding(.vertical, 8)
		.background(.bar)
	}

	private var placeholder: String {
		guard let post, let author = social.profile(post.authorId) else { return "Add a comment" }
		return "Add a comment for \(author.handle)"
	}

	private func send() {
		let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
		guard !text.isEmpty, !sending else { return }
		sending = true
		errorMessage = nil
		Task {
			defer { sending = false }
			do {
				try await social.addComment(postId, text: text)
				draft = ""
			} catch {
				errorMessage = friendlyMessage(error)
			}
		}
	}
}

private struct CommentRow: View {
	let profile: Profile?
	let text: String
	let date: Date

	var body: some View {
		HStack(alignment: .top, spacing: 12) {
			if let profile {
				NavigationLink(value: Route.profile(profile.id)) {
					AvatarView(profile: profile, size: 34)
				}
				.buttonStyle(.plain)
			}
			VStack(alignment: .leading, spacing: 3) {
				HStack(spacing: 6) {
					Text(profile?.handle ?? "")
						.font(.footnote.weight(.semibold))
					Text(Timestamp.short(date))
						.font(.footnote)
						.foregroundStyle(.secondary)
				}
				Text(text)
					.font(.subheadline)
					.fixedSize(horizontal: false, vertical: true)
			}
			Spacer(minLength: 0)
		}
	}
}
