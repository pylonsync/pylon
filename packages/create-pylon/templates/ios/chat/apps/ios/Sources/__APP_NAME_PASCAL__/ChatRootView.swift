import SwiftUI
import PylonClient
import PylonSync
import PylonSwiftUI

/// Two-pane chat: rooms list → room view. Subscribes to Room
/// inserts/deletes via PylonQuery — every new room created on any
/// device shows up here within ~ms over the WebSocket sync channel.
struct ChatRootView: View {
	@EnvironmentObject var session: AppSession
	let engine: SyncEngine
	@StateObject private var rooms: PylonQuery<Room>
	@State private var errorMessage: String?

	init(engine: SyncEngine) {
		self.engine = engine
		_rooms = StateObject(
			wrappedValue: PylonQuery<Room>(engine: engine, entity: "Room"),
		)
	}

	var body: some View {
		NavigationStack {
			List {
				Section("Your name") {
					TextField("display name", text: Binding(
						get: { session.authorName },
						set: { session.setAuthorName($0) },
					))
					.autocorrectionDisabled()
				}
				Section("Rooms") {
					if rooms.rows.isEmpty {
						Text("No rooms yet — create one below.")
							.foregroundStyle(.secondary)
					} else {
						ForEach(sortedRooms) { r in
							NavigationLink(value: r) {
								VStack(alignment: .leading) {
									Text(r.name)
									Text("#\(r.slug)")
										.font(.system(.caption, design: .monospaced))
										.foregroundStyle(.secondary)
								}
							}
						}
					}
				}
				Section {
					Button("Create room") {
						Task { await createRoom() }
					}
				}
				if let errorMessage {
					Section {
						Text(errorMessage)
							.foregroundStyle(.red)
							.font(.caption)
					}
				}
			}
			.navigationTitle("__APP_NAME__")
			.navigationDestination(for: Room.self) { room in
				RoomView(room: room, engine: engine)
			}
		}
	}

	private var sortedRooms: [Room] {
		rooms.rows.sorted { $0.createdAt < $1.createdAt }
	}

	private func createRoom() async {
		let name = await prompt(title: "Create room", message: "Room name?")
		guard let name = name?.trimmingCharacters(in: .whitespaces),
			!name.isEmpty
		else { return }
		let slug = name.lowercased()
			.replacingOccurrences(of: "[^a-z0-9]+", with: "-", options: .regularExpression)
			.trimmingCharacters(in: CharacterSet(charactersIn: "-"))
		do {
			// Server validates + writes via ctx.db.insert. The change_event
			// flows back through the SyncEngine and PylonQuery picks up
			// the new Room without us touching local state.
			let _: Room = try await session.client.callFn(
				"createRoom",
				args: CreateRoomArgs(slug: slug, name: name),
			)
		} catch {
			errorMessage = "Create failed: \(error.localizedDescription)"
		}
	}

	@MainActor
	private func prompt(title: String, message: String) async -> String? {
#if canImport(UIKit)
		await withCheckedContinuation { (cont: CheckedContinuation<String?, Never>) in
			let alert = UIAlertController(title: title, message: message, preferredStyle: .alert)
			alert.addTextField()
			alert.addAction(UIAlertAction(title: "Cancel", style: .cancel) { _ in cont.resume(returning: nil) })
			alert.addAction(UIAlertAction(title: "Create", style: .default) { _ in cont.resume(returning: alert.textFields?.first?.text) })
			UIApplication.shared.connectedScenes
				.compactMap { ($0 as? UIWindowScene)?.windows.first }
				.first?
				.rootViewController?
				.present(alert, animated: true)
		}
#else
		return nil
#endif
	}
}

#if canImport(UIKit)
import UIKit
#endif
