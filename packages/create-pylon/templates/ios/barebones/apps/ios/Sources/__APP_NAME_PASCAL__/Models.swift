import Foundation

/// Mirrors `Widget` from `apps/api/schema.ts`. Kept hand-written here
/// for clarity in the scaffold; for production, run
/// `pylon codegen client schema.ts --target swift --out Models.swift`
/// from `apps/api/` to regenerate this file from the schema.
struct Widget: Codable, Identifiable, Hashable {
	let id: String
	let name: String
	let count: Int
	let createdAt: String
}

/// Argument shape for `createWidget`.
struct CreateWidgetArgs: Encodable {
	let name: String
}
