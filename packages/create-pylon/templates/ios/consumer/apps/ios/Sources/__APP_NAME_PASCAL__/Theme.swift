import SwiftUI

enum Theme {
	/// Filled buttons, links, and badges. Fixed so the black tab bar tint
	/// does not change it.
	static let accent = Color(red: 0.0, green: 0.584, blue: 0.965)
	/// The like color.
	static let heart = Color(red: 1.0, green: 0.19, blue: 0.25)
	/// The ring around an avatar with a post from the last 24 hours.
	static let storyRing = AngularGradient(
		colors: [
			Color(red: 0.99, green: 0.73, blue: 0.27),
			Color(red: 0.98, green: 0.38, blue: 0.20),
			Color(red: 0.87, green: 0.15, blue: 0.47),
			Color(red: 0.55, green: 0.22, blue: 0.85),
			Color(red: 0.99, green: 0.73, blue: 0.27),
		],
		center: .center
	)
	static let hairline = Color(uiColor: .separator)
	static let fieldFill = Color(uiColor: .secondarySystemBackground)
}

/// The app name in the feed header and on the sign-in screen.
struct Wordmark: View {
	var size: CGFloat = 30

	var body: some View {
		Text("__APP_NAME__")
			.font(.system(size: size, weight: .semibold, design: .serif))
			.italic()
			.tracking(-0.5)
	}
}

/// A full-width filled button for the main action on a screen.
struct PrimaryButtonStyle: ButtonStyle {
	var isLoading = false

	func makeBody(configuration: Configuration) -> some View {
		ZStack {
			configuration.label.opacity(isLoading ? 0 : 1)
			if isLoading { ProgressView().tint(.white) }
		}
		.font(.body.weight(.semibold))
		.foregroundStyle(.white)
		.frame(maxWidth: .infinity, minHeight: 50)
		.background(Theme.accent, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
		.opacity(configuration.isPressed ? 0.8 : 1)
	}
}

/// A compact button for profile actions: Edit profile, Follow, Following.
struct ProfileButtonStyle: ButtonStyle {
	var prominent = false

	func makeBody(configuration: Configuration) -> some View {
		configuration.label
			.font(.subheadline.weight(.semibold))
			.foregroundStyle(prominent ? Color.white : Color.primary)
			.frame(maxWidth: .infinity, minHeight: 34)
			.background(
				prominent ? Theme.accent : Theme.fieldFill,
				in: RoundedRectangle(cornerRadius: 9, style: .continuous)
			)
			.opacity(configuration.isPressed ? 0.7 : 1)
	}
}

/// A rounded text field on the sign-in and profile screens.
struct FieldStyle: ViewModifier {
	func body(content: Content) -> some View {
		content
			.padding(.horizontal, 14)
			.frame(minHeight: 50)
			.background(Theme.fieldFill, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
	}
}

extension View {
	func fieldStyle() -> some View { modifier(FieldStyle()) }
}
