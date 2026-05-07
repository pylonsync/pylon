import Foundation

/// Mirrors the `myOrgs` query response shape — Org row + the
/// caller's role. For production, codegen from schema.ts with
/// `pylon codegen client --target swift`.
struct Org: Codable, Identifiable, Hashable {
	let id: String
	let slug: String
	let name: String
	let role: String
	let createdAt: String
}

struct CreateOrgArgs: Encodable { let slug: String; let name: String }
struct EmptyArgs: Encodable {}
