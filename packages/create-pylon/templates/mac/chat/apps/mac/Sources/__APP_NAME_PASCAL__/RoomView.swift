import SwiftUI
import PylonClient

struct RoomView: View {
	@EnvironmentObject var session: AppSession
	let room: Room
	@State private var messages: [Message] = []
	@State private var draft = ""
	@State private var sending = false
	@State private var errorMessage: String?
	@State private var pollTimer: Task<Void, Never>?

	var body: some View {
		VStack(spacing: 0) {
			ScrollViewReader { proxy in
				ScrollView {
					LazyVStack(alignment: .leading, spacing: 12) {
						ForEach(messages) { msg in
							messageRow(msg).id(msg.id)
						}
						if messages.isEmpty {
							Text("No messages yet. Say hi.")
								.foregroundStyle(.secondary)
								.padding(.top, 32)
								.frame(maxWidth: .infinity)
						}
					}
					.padding(16)
				}
				.onChange(of: messages.count) { _, _ in
					if let last = messages.last {
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
		.task {
			await loadMessages()
			startPolling()
		}
		.onDisappear { pollTimer?.cancel() }
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

	private func loadMessages() async {
		do {
			messages = try await session.pylon.callFn(
				"roomMessages",
				args: RoomMessagesArgs(roomId: room.id),
			)
			errorMessage = nil
		} catch {
			errorMessage = "Load failed: \(error.localizedDescription)"
		}
	}

	private func startPolling() {
		pollTimer?.cancel()
		pollTimer = Task {
			while !Task.isCancelled {
				try? await Task.sleep(nanoseconds: 1_500_000_000)
				if Task.isCancelled { break }
				await loadMessages()
			}
		}
	}

	private func send() async {
		let body = draft.trimmingCharacters(in: .whitespaces)
		guard !body.isEmpty else { return }
		sending = true
		defer { sending = false }
		do {
			let msg: Message = try await session.pylon.callFn(
				"sendMessage",
				args: SendMessageArgs(
					roomId: room.id,
					body: body,
					authorName: session.authorName,
				),
			)
			messages.append(msg)
			draft = ""
			errorMessage = nil
		} catch {
			errorMessage = "Send failed: \(error.localizedDescription)"
		}
	}
}
