import Foundation

/// Mirrors `Todo` from `apps/api/schema.ts`. Regenerate from the
/// schema with `pylon codegen client schema.ts --target swift` for
/// production.
struct Todo: Codable, Identifiable, Hashable {
	let id: String
	var title: String
	var done: Bool
	let createdAt: String
	var position: Double?
}

struct AddTodoArgs: Encodable { let title: String }
struct ToggleTodoArgs: Encodable { let id: String; let done: Bool }
struct EditTodoArgs: Encodable { let id: String; let title: String }
struct DeleteTodoArgs: Encodable { let id: String }
struct ReorderTodoArgs: Encodable { let id: String; let position: Double }
struct EmptyArgs: Encodable {}
