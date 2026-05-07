import SwiftUI
import PylonClient

/// Lists every Widget and lets the user create a new one. Uses
/// PylonClient's HTTP API directly — no offline mirror, no realtime
/// subscription. Plenty for a barebones starter; upgrade to
/// `PylonQuery` from `PylonSwiftUI` when you need live updates.
struct ContentView: View {
	@EnvironmentObject var session: AppSession

	@State private var widgets: [Widget] = []
	@State private var newName: String = ""
	@State private var loading = true
	@State private var creating = false
	@State private var errorMessage: String?

	var body: some View {
		NavigationStack {
			List {
				Section("Create") {
					HStack {
						TextField("Name a widget…", text: $newName)
							.textFieldStyle(.roundedBorder)
							.autocorrectionDisabled()
						Button("Add") { Task { await create() } }
							.buttonStyle(.borderedProminent)
							.disabled(newName.trimmingCharacters(in: .whitespaces).isEmpty || creating)
					}
				}

				Section("Widgets") {
					if loading {
						ProgressView()
					} else if widgets.isEmpty {
						Text("No widgets yet.")
							.foregroundStyle(.secondary)
					} else {
						ForEach(widgets) { w in
							HStack {
								Text(w.name).font(.body)
								Spacer()
								Text("count: \(w.count)")
									.font(.system(.caption, design: .monospaced))
									.foregroundStyle(.secondary)
							}
						}
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
			.task { await load() }
			.refreshable { await load() }
		}
	}

	private func load() async {
		loading = true
		defer { loading = false }
		do {
			let rows: [Widget] = try await session.pylon.callFn(
				"listWidgets",
				args: EmptyArgs(),
			)
			widgets = rows
			errorMessage = nil
		} catch {
			errorMessage = "Load failed: \(error.localizedDescription)"
		}
	}

	private func create() async {
		let trimmed = newName.trimmingCharacters(in: .whitespaces)
		guard !trimmed.isEmpty else { return }
		creating = true
		defer { creating = false }
		do {
			let widget: Widget = try await session.pylon.callFn(
				"createWidget",
				args: CreateWidgetArgs(name: trimmed),
			)
			widgets.insert(widget, at: 0)
			newName = ""
			errorMessage = nil
		} catch {
			errorMessage = "Create failed: \(error.localizedDescription)"
		}
	}
}

private struct EmptyArgs: Encodable {}
