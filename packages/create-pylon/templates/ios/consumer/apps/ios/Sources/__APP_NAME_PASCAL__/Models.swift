import Foundation

// Row shapes for the entities in apps/api/app.ts. The sync engine hands
// every row over as JSON, so optional fields must be optional here too.

struct Profile: Codable, Identifiable, Hashable {
	let id: String
	let userId: String
	let handle: String
	let displayName: String
	let bio: String?
	let avatarUrl: String?
	let createdAt: String
}

struct Post: Codable, Identifiable, Hashable {
	let id: String
	let authorId: String
	let imageUrl: String
	let caption: String?
	let createdAt: String

	var date: Date { Timestamp.parse(createdAt) }
}

struct Like: Codable, Identifiable, Hashable {
	let id: String
	let postId: String
	let profileId: String
	let createdAt: String
}

struct Comment: Codable, Identifiable, Hashable {
	let id: String
	let postId: String
	let profileId: String
	let text: String
	let createdAt: String

	var date: Date { Timestamp.parse(createdAt) }
}

struct Follow: Codable, Identifiable, Hashable {
	let id: String
	let followerId: String
	let followingId: String
	let createdAt: String
}

// MARK: - Function arguments and results

struct EmptyArgs: Encodable {}
struct SeedResult: Decodable { let seeded: Bool }
struct IdResult: Decodable { let id: String }
struct UpsertProfileArgs: Encodable {
	let handle: String
	let displayName: String
	let bio: String
	let avatarUrl: String?
}
struct CreatePostArgs: Encodable {
	let imageUrl: String
	let caption: String
}
struct SetLikeArgs: Encodable {
	let postId: String
	let liked: Bool
}
struct DeletePostArgs: Encodable { let id: String }
struct AddCommentArgs: Encodable {
	let postId: String
	let text: String
}
struct SetFollowArgs: Encodable {
	let profileId: String
	let following: Bool
}
struct SetLikeResult: Decodable {
	let liked: Bool
	let likeCount: Int
}
struct SetFollowResult: Decodable { let following: Bool }

// MARK: - Timestamps

enum Timestamp {
	private static let withFraction: ISO8601DateFormatter = {
		let f = ISO8601DateFormatter()
		f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
		return f
	}()
	private static let plain = ISO8601DateFormatter()

	static func parse(_ iso: String) -> Date {
		withFraction.date(from: iso) ?? plain.date(from: iso) ?? .distantPast
	}

	/// "now", "5m", "2h", "3d", "2w", then "Sep 3".
	static func short(_ date: Date, now: Date = .now) -> String {
		let seconds = max(0, now.timeIntervalSince(date))
		switch seconds {
		case ..<60: return "now"
		case ..<3_600: return "\(Int(seconds / 60))m"
		case ..<86_400: return "\(Int(seconds / 3_600))h"
		case ..<604_800: return "\(Int(seconds / 86_400))d"
		case ..<2_419_200: return "\(Int(seconds / 604_800))w"
		default: return date.formatted(.dateTime.month(.abbreviated).day())
		}
	}

	/// "Just now", "5 minutes ago", "2 hours ago", "3 days ago", then "September 3".
	static func long(_ date: Date, now: Date = .now) -> String {
		let seconds = max(0, now.timeIntervalSince(date))
		func unit(_ n: Int, _ name: String) -> String { "\(n) \(name)\(n == 1 ? "" : "s") ago" }
		switch seconds {
		case ..<60: return "Just now"
		case ..<3_600: return unit(Int(seconds / 60), "minute")
		case ..<86_400: return unit(Int(seconds / 3_600), "hour")
		case ..<604_800: return unit(Int(seconds / 86_400), "day")
		default: return date.formatted(.dateTime.month(.wide).day())
		}
	}
}

extension Int {
	/// 950 → "950", 1_204 → "1,204", 12_400 → "12.4K".
	var compactCount: String {
		if self < 10_000 { return formatted(.number) }
		return formatted(.number.notation(.compactName).precision(.fractionLength(0...1)))
	}
}
