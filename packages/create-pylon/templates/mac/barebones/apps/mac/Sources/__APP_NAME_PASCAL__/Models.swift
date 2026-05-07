import Foundation

/// Mirrors `Widget` from `apps/api/schema.ts`. Regenerate from the
/// schema with `pylon codegen client schema.ts --target swift` for
/// production.
struct Widget: Codable, Identifiable, Hashable {
	let id: String
	let name: String
	let count: Int
	let createdAt: String
}

struct CreateWidgetArgs: Encodable {
	let name: String
}

struct EmptyArgs: Encodable {}
