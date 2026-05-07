import Foundation

struct Profile: Codable, Identifiable, Hashable {
	let id: String
	let userId: String
	let handle: String
	let displayName: String
	let bio: String?
	let createdAt: String
}

struct FeedItem: Codable, Identifiable, Hashable {
	let id: String
	let body: String
	let createdAt: String
	let author: AuthorCard?
	var likeCount: Int
	var likedByMe: Bool
}

struct AuthorCard: Codable, Hashable {
	let id: String
	let handle: String
	let displayName: String
}

struct UpsertProfileArgs: Encodable {
	let handle: String
	let displayName: String
	let bio: String
}

struct CreatePostArgs: Encodable { let body: String }
struct DeletePostArgs: Encodable { let id: String }
struct ToggleLikeArgs: Encodable { let postId: String }

struct ToggleLikeResult: Codable {
	let liked: Bool
	let likeCount: Int
}

struct EmptyArgs: Encodable {}
