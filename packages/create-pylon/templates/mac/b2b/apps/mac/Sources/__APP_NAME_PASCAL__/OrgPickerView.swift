import SwiftUI
import PylonClient

/// macOS org picker — sidebar with every Org the user belongs to,
/// detail panel for the active one. Mirrors the web `OrgPicker`
/// component; the same `myOrgs` query backs both.
struct OrgPickerView: View {
	@EnvironmentObject var session: AppSession
	@State private var orgs: [Org] = []
	@State private var selected: Org.ID?
	@State private var loading = true
	@State private var creating = false
	@State private var draftName = ""
	@State private var draftSlug = ""
	@State private var pending = false
	@State private var errorMessage: String?

	var body: some View {
		NavigationSplitView {
			List(selection: $selected) {
				Section {
					ForEach(orgs) { o in
						HStack {
							VStack(alignment: .leading, spacing: 2) {
								Text(o.name).font(.body)
								Text(o.slug)
									.font(.system(.caption, design: .monospaced))
									.foregroundStyle(.secondary)
							}
							Spacer()
							Text(o.role.uppercased())
								.font(.system(.caption, design: .monospaced))
								.foregroundStyle(roleColor(o.role))
						}
						.tag(o.id)
					}
				} header: {
					Text("Your organizations")
				}
			}
			.toolbar {
				ToolbarItem(placement: .primaryAction) {
					Button {
						creating.toggle()
					} label: {
						Image(systemName: "plus")
					}
				}
			}
		} detail: {
			if creating {
				createForm
			} else if let id = selected, let org = orgs.first(where: { $0.id == id }) {
				detail(org: org)
			} else {
				placeholder
			}
		}
		.task { await load() }
	}

	private var createForm: some View {
		VStack(alignment: .leading, spacing: 16) {
			Text("Create an organization")
				.font(.title3)
				.fontWeight(.semibold)
			VStack(alignment: .leading, spacing: 8) {
				TextField("Org name", text: $draftName)
					.textFieldStyle(.roundedBorder)
				TextField("Slug (e.g. acme-corp)", text: $draftSlug)
					.textFieldStyle(.roundedBorder)
					.autocorrectionDisabled()
			}
			HStack {
				Button("Cancel") {
					creating = false
					draftName = ""
					draftSlug = ""
					errorMessage = nil
				}
				.buttonStyle(.bordered)
				Button("Create") { Task { await create() } }
					.keyboardShortcut(.defaultAction)
					.disabled(draftName.trimmingCharacters(in: .whitespaces).isEmpty
						|| draftSlug.trimmingCharacters(in: .whitespaces).isEmpty
						|| pending)
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

	@ViewBuilder
	private func detail(org: Org) -> some View {
		VStack(alignment: .leading, spacing: 16) {
			Text(org.name).font(.title2).fontWeight(.semibold)
			Text(org.slug)
				.font(.system(.body, design: .monospaced))
				.foregroundStyle(.secondary)
			Divider()
			Text("Your role: \(org.role)")
				.foregroundStyle(.secondary)
				.font(.callout)
			Text("orgId: \(org.id)")
				.foregroundStyle(.tertiary)
				.font(.system(.caption, design: .monospaced))
			Spacer()
			Text("Wire member management + project lists into this panel by calling /api/fn/orgMembers and /api/fn/orgProjects with this orgId. Membership policies enforce tenant isolation server-side.")
				.foregroundStyle(.secondary)
				.font(.caption)
				.fixedSize(horizontal: false, vertical: true)
		}
		.padding(24)
	}

	private var placeholder: some View {
		VStack(spacing: 8) {
			if loading {
				ProgressView()
			} else if orgs.isEmpty {
				Text("You're not in any orgs yet.")
				Button("Create your first org") { creating = true }
					.buttonStyle(.bordered)
			} else {
				Text("Pick an org from the sidebar.")
					.foregroundStyle(.secondary)
			}
		}
		.frame(maxWidth: .infinity, maxHeight: .infinity)
	}

	private func roleColor(_ role: String) -> Color {
		switch role {
		case "owner": return .blue
		case "admin": return .orange
		default: return .secondary
		}
	}

	private func load() async {
		loading = true
		defer { loading = false }
		do {
			orgs = try await session.pylon.callFn("myOrgs", args: EmptyArgs())
			if selected == nil { selected = orgs.first?.id }
			errorMessage = nil
		} catch {
			errorMessage = "Load failed: \(error.localizedDescription)"
		}
	}

	private func create() async {
		let name = draftName.trimmingCharacters(in: .whitespaces)
		let slug = draftSlug.trimmingCharacters(in: .whitespaces).lowercased()
		guard !name.isEmpty, !slug.isEmpty else { return }
		pending = true
		defer { pending = false }
		do {
			let org: Org = try await session.pylon.callFn(
				"createOrg",
				args: CreateOrgArgs(slug: slug, name: name),
			)
			orgs.insert(org, at: 0)
			selected = org.id
			creating = false
			draftName = ""
			draftSlug = ""
			errorMessage = nil
		} catch {
			errorMessage = "Create failed: \(error.localizedDescription)"
		}
	}
}
