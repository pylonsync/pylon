import Foundation

struct Profile: Codable, Identifiable, Hashable {
	let id: String
	let userId: String
	let handle: String
	let displayName: String
	let bio: String?
	let createdAt: String
}

struct Post: Codable, Identifiable, Hashable {
	let id: String
	let authorId: String
	let body: String
	let createdAt: String
}

struct Like: Codable, Identifiable, Hashable {
	let id: String
	let postId: String
	let profileId: String
	let createdAt: String
}

struct UpsertProfileArgs: Encodable {
	let handle: String
	let displayName: String
	let bio: String
}
struct CreatePostArgs: Encodable { let body: String }
struct DeletePostArgs: Encodable { let id: String }
struct ToggleLikeArgs: Encodable { let postId: String }
struct EmptyArgs: Encodable {}
