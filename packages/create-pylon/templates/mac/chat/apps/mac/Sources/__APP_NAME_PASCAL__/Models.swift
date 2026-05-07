import Foundation

struct Room: Codable, Identifiable, Hashable {
	let id: String
	let slug: String
	let name: String
	let createdAt: String
}

struct Message: Codable, Identifiable, Hashable {
	let id: String
	let roomId: String
	let authorId: String
	let authorName: String
	let body: String
	let createdAt: String
}

struct CreateRoomArgs: Encodable { let slug: String; let name: String }
struct RoomMessagesArgs: Encodable { let roomId: String }
struct SendMessageArgs: Encodable {
	let roomId: String
	let body: String
	let authorName: String
}
struct EmptyArgs: Encodable {}
