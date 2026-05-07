import SwiftUI
import PylonClient
import PylonSync
import PylonSwiftUI

/// macOS chat: rooms in a sidebar, room view in the detail pane.
/// PylonQuery<Room> subscribes via the SyncEngine — new rooms from
/// any client appear without polling. Same story inside RoomView for
/// messages.
struct ChatRootView: View {
	@EnvironmentObject var session: AppSession
	let engine: SyncEngine
	@StateObject private var rooms: PylonQuery<Room>
	@State private var selected: Room.ID?
	@State private var showingCreate = false
	@State private var draftSlug = ""
	@State private var draftName = ""
	@State private var errorMessage: String?

	init(engine: SyncEngine) {
		self.engine = engine
		_rooms = StateObject(
			wrappedValue: PylonQuery<Room>(engine: engine, entity: "Room"),
		)
	}

	var body: some View {
		NavigationSplitView {
			VStack(spacing: 0) {
				List(selection: $selected) {
					Section("Rooms") {
						ForEach(sortedRooms) { r in
							VStack(alignment: .leading, spacing: 2) {
								Text(r.name)
								Text("#\(r.slug)")
									.font(.system(.caption, design: .monospaced))
									.foregroundStyle(.secondary)
							}
							.tag(r.id)
						}
					}
				}
				Divider()
				HStack(spacing: 8) {
					TextField("display name", text: Binding(
						get: { session.authorName },
						set: { session.setAuthorName($0) },
					))
					.textFieldStyle(.roundedBorder)
					.font(.caption)
				}
				.padding(8)
			}
			.toolbar {
				ToolbarItem(placement: .primaryAction) {
					Button {
						showingCreate.toggle()
					} label: {
						Image(systemName: "plus")
					}
				}
			}
			.sheet(isPresented: $showingCreate) {
				createSheet
			}
		} detail: {
			if let id = selected, let room = sortedRooms.first(where: { $0.id == id }) {
				RoomView(room: room, engine: engine)
			} else {
				placeholder
			}
		}
		.onAppear {
			if selected == nil { selected = sortedRooms.first?.id }
		}
		.onChange(of: rooms.rows.count) { _, _ in
			if selected == nil { selected = sortedRooms.first?.id }
		}
	}

	private var sortedRooms: [Room] {
		rooms.rows.sorted { $0.createdAt < $1.createdAt }
	}

	private var createSheet: some View {
		VStack(alignment: .leading, spacing: 12) {
			Text("Create a room").font(.headline)
			TextField("Name", text: $draftName)
				.textFieldStyle(.roundedBorder)
			TextField("Slug (e.g. general)", text: $draftSlug)
				.textFieldStyle(.roundedBorder)
				.autocorrectionDisabled()
			HStack {
				Spacer()
				Button("Cancel") { showingCreate = false }
					.buttonStyle(.bordered)
				Button("Create") { Task { await create() } }
					.keyboardShortcut(.defaultAction)
					.disabled(draftName.trimmingCharacters(in: .whitespaces).isEmpty
						|| draftSlug.trimmingCharacters(in: .whitespaces).isEmpty)
			}
			if let errorMessage {
				Text(errorMessage).foregroundStyle(.red).font(.caption)
			}
		}
		.padding(20)
		.frame(width: 360)
	}

	private var placeholder: some View {
		VStack(spacing: 8) {
			if rooms.rows.isEmpty {
				Text("No rooms yet.")
					.foregroundStyle(.secondary)
				Button("Create your first room") { showingCreate = true }
					.buttonStyle(.bordered)
			} else {
				Text("Pick a room from the sidebar.")
					.foregroundStyle(.secondary)
			}
		}
		.frame(maxWidth: .infinity, maxHeight: .infinity)
	}

	private func create() async {
		let name = draftName.trimmingCharacters(in: .whitespaces)
		let slug = draftSlug.trimmingCharacters(in: .whitespaces).lowercased()
		guard !name.isEmpty, !slug.isEmpty else { return }
		do {
			let room: Room = try await session.client.callFn(
				"createRoom",
				args: CreateRoomArgs(slug: slug, name: name),
			)
			selected = room.id
			showingCreate = false
			draftName = ""
			draftSlug = ""
			errorMessage = nil
		} catch {
			errorMessage = "Create failed: \(error.localizedDescription)"
		}
	}
}
