import SwiftUI
import PylonClient

/// Two-pane chat: rooms list → room view. Polls the active room
/// every 1.5s. For realtime, swap `pollMessages()` out for a
/// PylonQuery<Message> from PylonSwiftUI subscribed by roomId.
struct ChatRootView: View {
	@EnvironmentObject var session: AppSession
	@State private var rooms: [Room] = []
	@State private var loadingRooms = true
	@State private var errorMessage: String?

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
					if loadingRooms {
						ProgressView()
					} else if rooms.isEmpty {
						Text("No rooms yet. Create one below.")
							.foregroundStyle(.secondary)
					} else {
						ForEach(rooms) { r in
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
				RoomView(room: room)
			}
			.task { await loadRooms() }
			.refreshable { await loadRooms() }
		}
	}

	private func loadRooms() async {
		loadingRooms = true
		defer { loadingRooms = false }
		do {
			rooms = try await session.pylon.callFn("listRooms", args: EmptyArgs())
			errorMessage = nil
		} catch {
			errorMessage = "Load failed: \(error.localizedDescription)"
		}
	}

	private func createRoom() async {
		let alert = await prompt(title: "Create room", message: "Room name?")
		guard let name = alert?.trimmingCharacters(in: .whitespaces),
			!name.isEmpty
		else { return }
		let slug = name.lowercased()
			.replacingOccurrences(of: "[^a-z0-9]+", with: "-", options: .regularExpression)
			.trimmingCharacters(in: CharacterSet(charactersIn: "-"))
		do {
			let room: Room = try await session.pylon.callFn(
				"createRoom",
				args: CreateRoomArgs(slug: slug, name: name),
			)
			rooms.append(room)
		} catch {
			errorMessage = "Create failed: \(error.localizedDescription)"
		}
	}

	@MainActor
	private func prompt(title: String, message: String) async -> String? {
		// Minimal SwiftUI prompt — for production replace with an
		// in-tree alert + TextField sheet. The scaffold uses a tiny
		// UIKit detour on iOS so the demo works without a custom
		// modal implementation.
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
