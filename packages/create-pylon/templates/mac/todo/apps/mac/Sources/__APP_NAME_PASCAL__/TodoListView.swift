import SwiftUI
import PylonClient

/// macOS Todo list. List with inline checkbox + edit-on-double-click +
/// delete on hover. macOS doesn't ship `EditButton`, so reordering is
/// drag-and-drop via `.onMove(perform:)` which works in editable List.
struct TodoListView: View {
	@EnvironmentObject var session: AppSession
	@State private var todos: [Todo] = []
	@State private var draftTitle: String = ""
	@State private var loading = true
	@State private var pending = false
	@State private var errorMessage: String?
	@State private var editingId: String?
	@State private var editingDraft: String = ""
	@State private var hoveredId: String?

	var body: some View {
		VStack(spacing: 0) {
			HStack(spacing: 8) {
				TextField("What needs doing?", text: $draftTitle)
					.textFieldStyle(.roundedBorder)
					.onSubmit { Task { await add() } }
				Button("Add") { Task { await add() } }
					.keyboardShortcut(.defaultAction)
					.disabled(draftTitle.trimmingCharacters(in: .whitespaces).isEmpty || pending)
			}
			.padding(16)

			Divider()

			if loading {
				ProgressView().padding()
				Spacer()
			} else if todos.isEmpty {
				Spacer()
				Text("No todos yet.")
					.foregroundStyle(.secondary)
				Spacer()
			} else {
				List {
					ForEach(todos) { todo in
						row(todo)
					}
					.onMove { source, destination in
						Task { await reorder(from: source, to: destination) }
					}
				}
				.listStyle(.inset)
			}

			if let errorMessage {
				Divider()
				Text(errorMessage)
					.foregroundStyle(.red)
					.font(.caption)
					.padding(8)
			}
		}
		.task { await load() }
		.toolbar {
			ToolbarItem(placement: .primaryAction) {
				Button { Task { await load() } } label: {
					Image(systemName: "arrow.clockwise")
				}
				.disabled(loading)
			}
		}
	}

	@ViewBuilder
	private func row(_ todo: Todo) -> some View {
		HStack(spacing: 10) {
			Button { Task { await toggle(todo) } } label: {
				Image(systemName: todo.done ? "checkmark.circle.fill" : "circle")
					.foregroundStyle(todo.done ? .green : .secondary)
					.imageScale(.large)
			}
			.buttonStyle(.plain)

			if editingId == todo.id {
				TextField("Title", text: $editingDraft)
					.textFieldStyle(.roundedBorder)
					.onSubmit { Task { await commitEdit(todo) } }
				Button("Save") { Task { await commitEdit(todo) } }
					.buttonStyle(.bordered)
				Button("Cancel") {
					editingId = nil
					editingDraft = ""
				}
				.buttonStyle(.bordered)
			} else {
				Text(todo.title)
					.strikethrough(todo.done, color: .secondary)
					.foregroundStyle(todo.done ? .secondary : .primary)
					.onTapGesture(count: 2) {
						editingId = todo.id
						editingDraft = todo.title
					}
				Spacer()
				if hoveredId == todo.id {
					Button("Edit") {
						editingId = todo.id
						editingDraft = todo.title
					}
					.buttonStyle(.borderless)
					.font(.caption)
					Button("Delete") { Task { await delete(todo) } }
						.buttonStyle(.borderless)
						.font(.caption)
						.foregroundStyle(.red)
				}
			}
		}
		.padding(.vertical, 4)
		.contentShape(Rectangle())
		.onHover { hovering in
			hoveredId = hovering ? todo.id : (hoveredId == todo.id ? nil : hoveredId)
		}
	}

	// MARK: - Network

	private func load() async {
		loading = true
		defer { loading = false }
		do {
			let rows: [Todo] = try await session.pylon.callFn("listTodos", args: EmptyArgs())
			todos = rows
			errorMessage = nil
		} catch {
			errorMessage = "Load failed: \(error.localizedDescription)"
		}
	}

	private func add() async {
		let trimmed = draftTitle.trimmingCharacters(in: .whitespaces)
		guard !trimmed.isEmpty else { return }
		pending = true
		defer { pending = false }
		do {
			let todo: Todo = try await session.pylon.callFn(
				"addTodo",
				args: AddTodoArgs(title: trimmed),
			)
			todos.append(todo)
			draftTitle = ""
		} catch {
			errorMessage = "Add failed: \(error.localizedDescription)"
		}
	}

	private func toggle(_ todo: Todo) async {
		let nextDone = !todo.done
		if let i = todos.firstIndex(where: { $0.id == todo.id }) {
			todos[i].done = nextDone
		}
		do {
			let _: Todo = try await session.pylon.callFn(
				"toggleTodo",
				args: ToggleTodoArgs(id: todo.id, done: nextDone),
			)
		} catch {
			if let i = todos.firstIndex(where: { $0.id == todo.id }) {
				todos[i].done = todo.done
			}
			errorMessage = "Toggle failed: \(error.localizedDescription)"
		}
	}

	private func commitEdit(_ todo: Todo) async {
		let trimmed = editingDraft.trimmingCharacters(in: .whitespaces)
		guard !trimmed.isEmpty, trimmed != todo.title else {
			editingId = nil
			editingDraft = ""
			return
		}
		let originalTitle = todo.title
		if let i = todos.firstIndex(where: { $0.id == todo.id }) {
			todos[i].title = trimmed
		}
		editingId = nil
		editingDraft = ""
		do {
			let _: Todo = try await session.pylon.callFn(
				"editTodo",
				args: EditTodoArgs(id: todo.id, title: trimmed),
			)
		} catch {
			if let i = todos.firstIndex(where: { $0.id == todo.id }) {
				todos[i].title = originalTitle
			}
			errorMessage = "Rename failed: \(error.localizedDescription)"
		}
	}

	private func delete(_ todo: Todo) async {
		let snapshot = todos
		todos.removeAll { $0.id == todo.id }
		do {
			let _: Todo = try await session.pylon.callFn(
				"deleteTodo",
				args: DeleteTodoArgs(id: todo.id),
			)
		} catch {
			todos = snapshot
			errorMessage = "Delete failed: \(error.localizedDescription)"
		}
	}

	private func reorder(from source: IndexSet, to destination: Int) async {
		var moved = todos
		moved.move(fromOffsets: source, toOffset: destination)
		guard let movedIndex = source.first else { return }
		let newIndex = destination > movedIndex ? destination - 1 : destination
		guard newIndex < moved.count else { return }
		let movedTodo = moved[newIndex]
		let prev = newIndex > 0 ? moved[newIndex - 1] : nil
		let next = newIndex + 1 < moved.count ? moved[newIndex + 1] : nil
		let prevPos = prev?.position ?? 0
		let nextPos = next?.position ?? 0
		let position: Double
		if prev != nil && next != nil {
			position = (prevPos + nextPos) / 2
		} else if prev != nil {
			position = prevPos + 1024
		} else if next != nil {
			position = nextPos - 1024
		} else {
			position = 1024
		}
		let snapshot = todos
		todos = moved
		do {
			let _: Todo = try await session.pylon.callFn(
				"reorderTodo",
				args: ReorderTodoArgs(id: movedTodo.id, position: position),
			)
		} catch {
			todos = snapshot
			errorMessage = "Reorder failed: \(error.localizedDescription)"
		}
	}
}
