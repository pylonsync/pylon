// swift-tools-version:5.9
import PackageDescription

// SwiftPM package for the __APP_NAME__ mobile app.
//
// The executable target runs on macOS via `swift run`. For an iOS
// build, generate an Xcode project from `project.yml`:
//
//     brew install xcodegen
//     xcodegen generate
//     open __APP_NAME_PASCAL__.xcodeproj
//
// The Xcode project pulls the same Sources/__APP_NAME_PASCAL__/ tree
// as `swift build`, so iOS + macOS share one source set.
let package = Package(
	name: "__APP_NAME_PASCAL__",
	platforms: [
		.iOS(.v16),
		.macOS(.v13),
	],
	dependencies: [
		.package(url: "https://github.com/pylonsync/pylon.git", from: "0.3.0"),
	],
	targets: [
		.executableTarget(
			name: "__APP_NAME_PASCAL__",
			dependencies: [
				.product(name: "PylonClient", package: "pylon"),
				.product(name: "PylonSwiftUI", package: "pylon"),
			],
			path: "Sources/__APP_NAME_PASCAL__"
		),
	]
)
