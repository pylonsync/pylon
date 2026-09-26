import SwiftUI

/// Email and password sign-in and sign-up against Pylon auth.
struct AuthView: View {
	@EnvironmentObject private var app: AppModel

	enum Mode { case signIn, signUp }
	enum Field { case email, password }

	@State private var mode: Mode = .signIn
	@State private var email = ""
	@State private var password = ""
	@State private var busy = false
	@State private var errorMessage: String?
	@State private var photos: [URL] = []
	@FocusState private var focus: Field?

	var body: some View {
		ZStack(alignment: .bottom) {
			PhotoMosaic(photos: photos)
				.ignoresSafeArea()
			// A fade that follows the form, so the form stays readable when
			// the keyboard pushes it up over the photos.
			form
				.padding(.horizontal, 24)
				.padding(.bottom, 12)
				.background(alignment: .top) {
					let base = Color(uiColor: .systemBackground)
					LinearGradient(
						stops: [
							.init(color: base.opacity(0), location: 0),
							.init(color: base.opacity(0.9), location: 0.22),
							.init(color: base, location: 0.3),
						],
						startPoint: .top,
						endPoint: .bottom
					)
					.padding(.top, -200)
					.ignoresSafeArea()
				}
		}
		.task { await loadPhotos() }
	}

	private var form: some View {
		VStack(spacing: 14) {
			VStack(spacing: 6) {
				Wordmark(size: 48)
				Text(mode == .signIn ? "Sign in to see photos from people you follow." : "Create an account to share your photos.")
					.font(.subheadline)
					.foregroundStyle(.secondary)
					.multilineTextAlignment(.center)
			}
			.padding(.bottom, 10)

			TextField("Email", text: $email)
				.textContentType(.emailAddress)
				.keyboardType(.emailAddress)
				.textInputAutocapitalization(.never)
				.autocorrectionDisabled()
				.submitLabel(.next)
				.focused($focus, equals: .email)
				.onSubmit { focus = .password }
				.fieldStyle()

			SecureField("Password", text: $password)
				.textContentType(mode == .signIn ? .password : .newPassword)
				.submitLabel(.go)
				.focused($focus, equals: .password)
				.onSubmit(submit)
				.fieldStyle()

			if let errorMessage {
				Text(errorMessage)
					.font(.footnote)
					.foregroundStyle(.red)
					.frame(maxWidth: .infinity, alignment: .leading)
					.transition(.opacity)
			}

			Button(mode == .signIn ? "Log in" : "Create account", action: submit)
				.buttonStyle(PrimaryButtonStyle(isLoading: busy))
				.disabled(busy || !canSubmit)
				.opacity(canSubmit ? 1 : 0.6)

			HStack(spacing: 4) {
				Text(mode == .signIn ? "No account yet?" : "Already have an account?")
					.foregroundStyle(.secondary)
				Button(mode == .signIn ? "Sign up" : "Log in") {
					withAnimation(.snappy) {
						mode = mode == .signIn ? .signUp : .signIn
						errorMessage = nil
					}
				}
				.fontWeight(.semibold)
			}
			.font(.footnote)
			.padding(.top, 6)
		}
		.animation(.snappy, value: errorMessage)
	}

	private var canSubmit: Bool {
		email.contains("@") && (mode == .signIn ? !password.isEmpty : password.count >= 8)
	}

	private func submit() {
		guard !busy else { return }
		guard canSubmit else {
			if mode == .signUp, password.count < 8 {
				errorMessage = "Use a password with at least 8 characters."
			}
			return
		}
		focus = nil
		busy = true
		errorMessage = nil
		let email = email.trimmingCharacters(in: .whitespaces).lowercased()
		Task {
			defer { busy = false }
			do {
				if mode == .signIn {
					try await app.signIn(email: email, password: password)
				} else {
					try await app.register(email: email, password: password)
				}
			} catch {
				errorMessage = friendlyMessage(error)
			}
		}
	}

	/// The newest public photos, for the background.
	private func loadPhotos() async {
		guard let posts = try? await app.client.list("Post", as: Post.self) else { return }
		photos = posts
			.sorted { $0.createdAt > $1.createdAt }
			.prefix(18)
			.compactMap { app.media.thumbnailURL(for: $0.imageUrl, width: 384) }
	}
}

/// Three columns of photos drifting upward at different speeds.
private struct PhotoMosaic: View {
	let photos: [URL]

	var body: some View {
		GeometryReader { geo in
			let spacing: CGFloat = 8
			let width = (geo.size.width - spacing * 4) / 3
			TimelineView(.animation) { timeline in
				let t = timeline.date.timeIntervalSinceReferenceDate
				HStack(alignment: .top, spacing: spacing) {
					ForEach(0..<3, id: \.self) { column in
						let items = columnPhotos(column)
						let tile = width * 1.25 + spacing
						let loop = tile * CGFloat(max(items.count, 1))
						let speed = [14.0, 20.0, 11.0][column]
						let offset = CGFloat((t * speed).truncatingRemainder(dividingBy: Double(max(loop, 1))))
						VStack(spacing: spacing) {
							ForEach(Array((items + items).enumerated()), id: \.offset) { _, url in
								RemoteImage(url: url)
									.frame(width: width, height: width * 1.25)
									.clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
							}
						}
						.offset(y: -offset - (column == 1 ? width * 0.5 : 0))
					}
				}
				.padding(.horizontal, spacing)
			}
		}
		.opacity(photos.isEmpty ? 0 : 1)
		.animation(.easeIn(duration: 0.6), value: photos.isEmpty)
		.allowsHitTesting(false)
		.accessibilityHidden(true)
	}

	private func columnPhotos(_ column: Int) -> [URL] {
		photos.enumerated().filter { $0.offset % 3 == column }.map(\.element)
	}
}
