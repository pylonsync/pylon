import SwiftUI
import PylonClient
import PylonSync
import PylonSwiftUI

/// Local Todo struct — would normally come from `PylonGenerated.swift`.
/// Inlined here so the example builds without a codegen step.
struct Todo: Codable, Identifiable, Equatable, Hashable {
    let id: String
    let userId: String
    var title: String
    var done: Bool
    var priority: String
    var notes: String?
    var dueAt: String?
    var completedAt: String?
    let createdAt: String
}

struct TodoListView: View {
    let engine: SyncEngine
    let client: PylonClient

    @StateObject private var todos: PylonQuery<Todo>
    @StateObject private var session: PylonSession

    @State private var newTitle = ""
    @State private var error: String?
    @EnvironmentObject var appSession: AppSession

    init(engine: SyncEngine, client: PylonClient) {
        self.engine = engine
        self.client = client
        _todos   = StateObject(wrappedValue: PylonQuery(engine: engine, entity: "Todo"))
        _session = StateObject(wrappedValue: PylonSession(engine: engine))
    }

    var body: some View {
        NavigationStack {
            List {
                Section {
                    HStack {
                        TextField("New todo", text: $newTitle)
                            .onSubmit { add() }
                        Button("Add") { add() }
                            .disabled(newTitle.isEmpty)
                    }
                }

                Section("Open") {
                    ForEach(todos.rows.filter { !$0.done }) { todo in
                        TodoRow(todo: todo, onToggle: { toggle(todo) })
                    }
                    .onDelete(perform: deleteOpen)
                }

                if !todos.rows.filter(\.done).isEmpty {
                    Section("Done") {
                        ForEach(todos.rows.filter(\.done)) { todo in
                            TodoRow(todo: todo, onToggle: { toggle(todo) })
                        }
                        .onDelete(perform: deleteDone)
                    }
                }
            }
            .navigationTitle("Todos")
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Menu {
                        if let userId = session.session.userId {
                            Text("Signed in as \(userId)")
                        }
                        Button("Sign out", role: .destructive) {
                            Task { await appSession.signOut() }
                        }
                    } label: {
                        Image(systemName: "person.circle")
                    }
                }
            }
            .alert("Error", isPresented: .constant(error != nil)) {
                Button("OK") { error = nil }
            } message: {
                Text(error ?? "")
            }
        }
    }

    private func add() {
        let title = newTitle.trimmingCharacters(in: .whitespaces)
        guard !title.isEmpty else { return }
        newTitle = ""
        Task {
            let userId = session.session.userId ?? ""
            let now = ISO8601DateFormatter().string(from: Date())
            // Optimistic insert via the sync engine.
            _ = await engine.insert("Todo", [
                "title":     .string(title),
                "userId":    .string(userId),
                "done":      .bool(false),
                "priority":  .string("normal"),
                "createdAt": .string(now),
            ])
        }
    }

    private func toggle(_ todo: Todo) {
        Task {
            let now = ISO8601DateFormatter().string(from: Date())
            await engine.update("Todo", id: todo.id, [
                "done":        .bool(!todo.done),
                "completedAt": todo.done ? .null : .string(now),
            ])
        }
    }

    private func deleteOpen(at offsets: IndexSet) {
        let openTodos = todos.rows.filter { !$0.done }
        for i in offsets {
            let todo = openTodos[i]
            Task { await engine.delete("Todo", id: todo.id) }
        }
    }

    private func deleteDone(at offsets: IndexSet) {
        let doneTodos = todos.rows.filter(\.done)
        for i in offsets {
            let todo = doneTodos[i]
            Task { await engine.delete("Todo", id: todo.id) }
        }
    }
}

struct TodoRow: View {
    let todo: Todo
    let onToggle: () -> Void

    var body: some View {
        Button(action: onToggle) {
            HStack {
                Image(systemName: todo.done ? "checkmark.circle.fill" : "circle")
                    .foregroundStyle(todo.done ? .green : .secondary)
                Text(todo.title)
                    .strikethrough(todo.done)
                    .foregroundStyle(todo.done ? .secondary : .primary)
                Spacer()
            }
        }
        .buttonStyle(.plain)
    }
}
