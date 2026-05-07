import SwiftUI
import PylonClient

/// macOS chat: rooms in a sidebar, room view in the detail pane.
/// Polls the active room every 1.5s. Replace with PylonQuery<Message>
/// from PylonSwiftUI for realtime push.
struct ChatRootView: View {
	@EnvironmentObject var session: AppSession
	@State private var rooms: [Room] = []
	@State private var selected: Room.ID?
	@State private var showingCreate = false
	@State private var draftSlug = ""
	@State private var draftName = ""
	@State private var loading = true
	@State private var errorMessage: String?

	var body: some View {
		NavigationSplitView {
			VStack(spacing: 0) {
				List(selection: $selected) {
					Section("Rooms") {
						ForEach(rooms) { r in
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
			if let id = selected, let room = rooms.first(where: { $0.id == id }) {
				RoomView(room: room)
			} else {
				placeholder
			}
		}
		.task { await load() }
	}

	private var createSheet: some View {
		VStack(alignment: .leading, spacing: 12) {
			Text("Create a room")
				.font(.headline)
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
			if loading {
				ProgressView()
			} else if rooms.isEmpty {
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

	private func load() async {
		loading = true
		defer { loading = false }
		do {
			rooms = try await session.pylon.callFn("listRooms", args: EmptyArgs())
			if selected == nil { selected = rooms.first?.id }
			errorMessage = nil
		} catch {
			errorMessage = "Load failed: \(error.localizedDescription)"
		}
	}

	private func create() async {
		let name = draftName.trimmingCharacters(in: .whitespaces)
		let slug = draftSlug.trimmingCharacters(in: .whitespaces).lowercased()
		guard !name.isEmpty, !slug.isEmpty else { return }
		do {
			let room: Room = try await session.pylon.callFn(
				"createRoom",
				args: CreateRoomArgs(slug: slug, name: name),
			)
			rooms.append(room)
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
