import Combine
import Foundation
import PylonClient
import PylonSync
import PylonSwiftUI

/// The live social graph. Five subscriptions (Profile, Post, Like, Comment,
/// Follow) on the sync engine, joined in memory. A row written by any
/// client reaches every open app over the engine's WebSocket, and every
/// view that reads this store redraws.
///
/// Writes go through server functions. The server writes the row, the
/// change comes back through sync, and the view updates. Likes and follows
/// also show at once through a pending state that clears when the synced
/// row arrives.
@MainActor
final class SocialStore: ObservableObject {
	let engine: SyncEngine
	let client: PylonClient
	let media: MediaService

	@Published var myProfileId: String?
	@Published private(set) var profiles: [String: Profile] = [:]
	/// Newest first.
	@Published private(set) var posts: [Post] = []
	@Published private(set) var likesByPost: [String: [Like]] = [:]
	/// Oldest first within a post.
	@Published private(set) var commentsByPost: [String: [Comment]] = [:]
	@Published private(set) var follows: [Follow] = []
	@Published private(set) var isLoading = true
	/// Post ids the user liked or unliked whose synced row has not arrived yet.
	@Published private(set) var pendingLikes: [String: Bool] = [:]
	@Published private(set) var pendingFollows: [String: Bool] = [:]

	private let profileQuery: PylonQuery<Profile>
	private let postQuery: PylonQuery<Post>
	private let likeQuery: PylonQuery<Like>
	private let commentQuery: PylonQuery<Comment>
	private let followQuery: PylonQuery<Follow>
	private var subscriptions: Set<AnyCancellable> = []

	init(engine: SyncEngine, client: PylonClient, media: MediaService) {
		self.engine = engine
		self.client = client
		self.media = media
		profileQuery = PylonQuery(engine: engine, entity: "Profile")
		postQuery = PylonQuery(engine: engine, entity: "Post")
		likeQuery = PylonQuery(engine: engine, entity: "Like")
		commentQuery = PylonQuery(engine: engine, entity: "Comment")
		followQuery = PylonQuery(engine: engine, entity: "Follow")

		profileQuery.$rows
			.sink { [weak self] rows in
				self?.profiles = Dictionary(rows.map { ($0.id, $0) }, uniquingKeysWith: { _, new in new })
			}
			.store(in: &subscriptions)
		postQuery.$rows
			.sink { [weak self] rows in
				self?.posts = rows.sorted { $0.createdAt > $1.createdAt }
			}
			.store(in: &subscriptions)
		likeQuery.$rows
			.sink { [weak self] rows in
				guard let self else { return }
				likesByPost = Dictionary(grouping: rows, by: \.postId)
				pendingLikes = pendingLikes.filter { postId, liked in
					self.syncedLike(postId) != liked
				}
			}
			.store(in: &subscriptions)
		commentQuery.$rows
			.sink { [weak self] rows in
				self?.commentsByPost = Dictionary(grouping: rows, by: \.postId)
					.mapValues { $0.sorted { $0.createdAt < $1.createdAt } }
			}
			.store(in: &subscriptions)
		followQuery.$rows
			.sink { [weak self] rows in
				guard let self else { return }
				follows = rows
				pendingFollows = pendingFollows.filter { profileId, following in
					self.syncedFollow(profileId) != following
				}
			}
			.store(in: &subscriptions)
		postQuery.$loading
			.sink { [weak self] loading in self?.isLoading = loading }
			.store(in: &subscriptions)
	}

	// MARK: - Reads

	var me: Profile? { myProfileId.flatMap { profiles[$0] } }

	func profile(_ id: String) -> Profile? { profiles[id] }

	func posts(by profileId: String) -> [Post] {
		posts.filter { $0.authorId == profileId }
	}

	func post(_ id: String) -> Post? { posts.first { $0.id == id } }

	func isLiked(_ postId: String) -> Bool {
		pendingLikes[postId] ?? syncedLike(postId)
	}

	func likeCount(_ postId: String) -> Int {
		let synced = likesByPost[postId]?.count ?? 0
		guard let pending = pendingLikes[postId], pending != syncedLike(postId) else { return synced }
		return synced + (pending ? 1 : -1)
	}

	/// A liker to name under the photo: someone the user follows first, else the latest.
	func featuredLiker(_ postId: String) -> Profile? {
		let likes = (likesByPost[postId] ?? []).filter { $0.profileId != myProfileId }
		let following = Set(followingIds(of: myProfileId))
		let pick = likes.first { following.contains($0.profileId) } ?? likes.max { $0.createdAt < $1.createdAt }
		return pick.flatMap { profiles[$0.profileId] }
	}

	func comments(_ postId: String) -> [Comment] { commentsByPost[postId] ?? [] }

	func followerIds(of profileId: String) -> [String] {
		follows.filter { $0.followingId == profileId }.map(\.followerId)
	}

	func followingIds(of profileId: String?) -> [String] {
		guard let profileId else { return [] }
		return follows.filter { $0.followerId == profileId }.map(\.followingId)
	}

	func isFollowing(_ profileId: String) -> Bool {
		pendingFollows[profileId] ?? syncedFollow(profileId)
	}

	func followerCount(_ profileId: String) -> Int {
		let synced = followerIds(of: profileId).count
		guard let pending = pendingFollows[profileId], pending != syncedFollow(profileId) else { return synced }
		return synced + (pending ? 1 : -1)
	}

	/// People with a post in the last 24 hours, the user first, then newest.
	var storyAuthors: [Profile] {
		let cutoff = Date.now.addingTimeInterval(-86_400)
		var seen = Set<String>()
		var out: [Profile] = []
		for post in posts where post.date > cutoff {
			guard !seen.contains(post.authorId), post.authorId != myProfileId,
			      let author = profiles[post.authorId] else { continue }
			seen.insert(post.authorId)
			out.append(author)
		}
		return out
	}

	func hasRecentPost(_ profileId: String) -> Bool {
		let cutoff = Date.now.addingTimeInterval(-86_400)
		return posts.contains { $0.authorId == profileId && $0.date > cutoff }
	}

	func recentPosts(by profileId: String) -> [Post] {
		let cutoff = Date.now.addingTimeInterval(-86_400)
		return posts.filter { $0.authorId == profileId && $0.date > cutoff }.reversed()
	}

	/// Likes, comments, and follows aimed at the user, newest first.
	var activity: [ActivityItem] {
		guard let me = myProfileId else { return [] }
		let mine = Set(posts.filter { $0.authorId == me }.map(\.id))
		var items: [ActivityItem] = []
		for postId in mine {
			for like in likesByPost[postId] ?? [] where like.profileId != me {
				items.append(ActivityItem(id: "like-\(like.id)", kind: .like, actorId: like.profileId, postId: postId, text: nil, createdAt: like.createdAt))
			}
			for comment in commentsByPost[postId] ?? [] where comment.profileId != me {
				items.append(ActivityItem(id: "comment-\(comment.id)", kind: .comment, actorId: comment.profileId, postId: postId, text: comment.text, createdAt: comment.createdAt))
			}
		}
		for follow in follows where follow.followingId == me {
			items.append(ActivityItem(id: "follow-\(follow.id)", kind: .follow, actorId: follow.followerId, postId: nil, text: nil, createdAt: follow.createdAt))
		}
		return items.sorted { $0.createdAt > $1.createdAt }
	}

	private func syncedLike(_ postId: String) -> Bool {
		guard let me = myProfileId else { return false }
		return likesByPost[postId]?.contains { $0.profileId == me } ?? false
	}

	private func syncedFollow(_ profileId: String) -> Bool {
		guard let me = myProfileId else { return false }
		return follows.contains { $0.followerId == me && $0.followingId == profileId }
	}

	// MARK: - Writes

	func toggleLike(_ postId: String) {
		let next = !isLiked(postId)
		setLike(postId, liked: next)
	}

	/// Double tap only ever likes.
	func like(_ postId: String) {
		if !isLiked(postId) { setLike(postId, liked: true) }
	}

	/// Records the tap at once, then sends the wanted state. The server call
	/// sets the state rather than flipping it, so taps that arrive out of
	/// order still end in the last tap's state.
	private func setLike(_ postId: String, liked: Bool) {
		pendingLikes[postId] = liked
		Task {
			do {
				_ = try await client.callFn("setLike", args: SetLikeArgs(postId: postId, liked: liked), as: SetLikeResult.self)
			} catch {
				if pendingLikes[postId] == liked { pendingLikes[postId] = nil }
			}
		}
	}

	func toggleFollow(_ profileId: String) {
		let following = !isFollowing(profileId)
		pendingFollows[profileId] = following
		Task {
			do {
				_ = try await client.callFn("setFollow", args: SetFollowArgs(profileId: profileId, following: following), as: SetFollowResult.self)
			} catch {
				if pendingFollows[profileId] == following { pendingFollows[profileId] = nil }
			}
		}
	}

	func addComment(_ postId: String, text: String) async throws {
		_ = try await client.callFn("addComment", args: AddCommentArgs(postId: postId, text: text), as: IdResult.self)
	}

	func deletePost(_ postId: String) async throws {
		_ = try await client.callFn("deletePost", args: DeletePostArgs(id: postId), as: IdResult.self)
	}

	func createPost(jpeg: Data, caption: String) async throws {
		let imageUrl = try await media.uploadPublicJPEG(jpeg)
		_ = try await client.callFn("createPost", args: CreatePostArgs(imageUrl: imageUrl, caption: caption), as: IdResult.self)
	}

	func saveProfile(handle: String, displayName: String, bio: String, avatarJPEG: Data?) async throws -> Profile {
		var avatarUrl: String?
		if let avatarJPEG {
			avatarUrl = try await media.uploadPublicJPEG(avatarJPEG)
		}
		return try await client.callFn(
			"upsertProfile",
			args: UpsertProfileArgs(handle: handle, displayName: displayName, bio: bio, avatarUrl: avatarUrl),
			as: Profile.self
		)
	}
}

struct ActivityItem: Identifiable, Hashable {
	enum Kind { case like, comment, follow }
	let id: String
	let kind: Kind
	let actorId: String
	let postId: String?
	let text: String?
	let createdAt: String

	var date: Date { Timestamp.parse(createdAt) }
}

/// A readable sentence for an error from a server function or the network.
func friendlyMessage(_ error: Error) -> String {
	if let pylon = error as? PylonError {
		switch pylon {
		case .http(_, _, let message?) where !message.isEmpty:
			return message.prefix(1).uppercased() + message.dropFirst()
		case .transport:
			return "Cannot reach the server. Check your connection and try again."
		default:
			return "Something went wrong. Try again."
		}
	}
	if let local = error as? LocalizedError, let text = local.errorDescription { return text }
	return "Something went wrong. Try again."
}
