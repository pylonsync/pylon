import SwiftUI
import PylonClient

/// macOS list + create form for Widget. Two-pane NavigationSplitView
/// with sidebar on the left and detail on the right — feels native on
/// macOS. Same backend as web/iOS/expo.
struct ContentView: View {
	@EnvironmentObject var session: AppSession
	@State private var widgets: [Widget] = []
	@State private var selected: Widget.ID?
	@State private var newName = ""
	@State private var loading = true
	@State private var creating = false
	@State private var errorMessage: String?

	var body: some View {
		NavigationSplitView {
			List(selection: $selected) {
				if widgets.isEmpty && !loading {
					Text("No widgets yet.")
						.foregroundStyle(.secondary)
						.font(.callout)
				} else {
					ForEach(widgets) { w in
						HStack {
							Text(w.name)
							Spacer()
							Text("\(w.count)")
								.font(.system(.caption, design: .monospaced))
								.foregroundStyle(.secondary)
						}
						.tag(w.id)
					}
				}
			}
			.navigationTitle("__APP_NAME__")
			.toolbar {
				ToolbarItem(placement: .primaryAction) {
					Button {
						Task { await load() }
					} label: {
						Image(systemName: "arrow.clockwise")
					}
					.disabled(loading)
				}
			}
		} detail: {
			VStack(alignment: .leading, spacing: 18) {
				Text("Create a widget")
					.font(.title3)
					.fontWeight(.semibold)
				HStack {
					TextField("Name…", text: $newName)
						.textFieldStyle(.roundedBorder)
						.onSubmit { Task { await create() } }
					Button("Create") { Task { await create() } }
						.keyboardShortcut(.defaultAction)
						.disabled(newName.trimmingCharacters(in: .whitespaces).isEmpty || creating)
				}
				if let errorMessage {
					Text(errorMessage)
						.foregroundStyle(.red)
						.font(.caption)
				}
				Spacer()
			}
			.padding(24)
		}
		.task { await load() }
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
