import SwiftUI
import PylonClient
import PylonSync
import PylonSwiftUI

/// Live room view. PylonQuery<Message> with a `where` predicate that
/// matches `roomId` to the active room. The sync engine pushes diffs
/// over WebSocket; new messages from any client land in `messages.rows`
/// without us polling.
struct RoomView: View {
	@EnvironmentObject var session: AppSession
	let room: Room
	let engine: SyncEngine
	@StateObject private var messages: PylonQuery<Message>
	@State private var draft = ""
	@State private var sending = false
	@State private var errorMessage: String?

	init(room: Room, engine: SyncEngine) {
		self.room = room
		self.engine = engine
		let roomId = room.id
		_messages = StateObject(
			wrappedValue: PylonQuery<Message>(
				engine: engine,
				entity: "Message",
				where: { row in
					row["roomId"]?.stringValue == roomId
				},
			),
		)
	}

	var body: some View {
		VStack(spacing: 0) {
			ScrollViewReader { proxy in
				ScrollView {
					LazyVStack(alignment: .leading, spacing: 12) {
						ForEach(sortedMessages) { msg in
							messageRow(msg).id(msg.id)
						}
						if messages.rows.isEmpty {
							Text("No messages yet. Say hi.")
								.foregroundStyle(.secondary)
								.padding(.top, 32)
								.frame(maxWidth: .infinity)
						}
					}
					.padding(16)
				}
				.onChange(of: messages.rows.count) { _, _ in
					if let last = sortedMessages.last {
						withAnimation { proxy.scrollTo(last.id, anchor: .bottom) }
					}
				}
			}

			if let errorMessage {
				Text(errorMessage)
					.foregroundStyle(.red)
					.font(.caption)
					.padding(8)
			}

			HStack {
				TextField("Message #\(room.slug)…", text: $draft, axis: .vertical)
					.textFieldStyle(.roundedBorder)
					.lineLimit(1...4)
					.onSubmit { Task { await send() } }
				Button("Send") { Task { await send() } }
					.keyboardShortcut(.defaultAction)
					.disabled(draft.trimmingCharacters(in: .whitespaces).isEmpty || sending)
			}
			.padding(12)
			.background(.thinMaterial)
		}
		.navigationTitle(room.name)
		#if os(iOS)
		.navigationBarTitleDisplayMode(.inline)
		#endif
	}

	private var sortedMessages: [Message] {
		messages.rows.sorted { $0.createdAt < $1.createdAt }
	}

	@ViewBuilder
	private func messageRow(_ msg: Message) -> some View {
		VStack(alignment: .leading, spacing: 2) {
			HStack(alignment: .firstTextBaseline) {
				Text(msg.authorName).font(.subheadline.weight(.medium))
				Text(formatTime(msg.createdAt))
					.font(.caption2)
					.foregroundStyle(.tertiary)
			}
			Text(msg.body)
				.font(.body)
				.fixedSize(horizontal: false, vertical: true)
		}
	}

	private func formatTime(_ iso: String) -> String {
		let formatter = ISO8601DateFormatter()
		formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
		guard let date = formatter.date(from: iso) ?? ISO8601DateFormatter().date(from: iso) else {
			return ""
		}
		let display = DateFormatter()
		display.timeStyle = .short
		return display.string(from: date)
	}

	private func send() async {
		let body = draft.trimmingCharacters(in: .whitespaces)
		guard !body.isEmpty else { return }
		sending = true
		defer { sending = false }
		do {
			// Server inserts via ctx.db.insert; the change_event flows
			// back through the SyncEngine and our PylonQuery picks up
			// the new row. We only clear the draft here.
			let _: Message = try await session.client.callFn(
				"sendMessage",
				args: SendMessageArgs(
					roomId: room.id,
					body: body,
					authorName: session.authorName,
				),
			)
			draft = ""
			errorMessage = nil
		} catch {
			errorMessage = "Send failed: \(error.localizedDescription)"
		}
	}
}
