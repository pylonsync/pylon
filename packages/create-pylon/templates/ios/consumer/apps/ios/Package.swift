// swift-tools-version:5.9
import PackageDescription

// SwiftPM manifest for the app sources. Xcode builds use project.yml
// (xcodegen); this file lets `swift build` resolve the Pylon packages.
let package = Package(
	name: "__APP_NAME_PASCAL__",
	platforms: [
		.iOS(.v17),
	],
	dependencies: [
		.package(url: "https://github.com/pylonsync/pylon-swift.git", from: "__PYLON_VERSION__"),
	],
	targets: [
		.executableTarget(
			name: "__APP_NAME_PASCAL__",
			dependencies: [
				.product(name: "PylonClient", package: "pylon-swift"),
				.product(name: "PylonSync", package: "pylon-swift"),
				.product(name: "PylonSwiftUI", package: "pylon-swift"),
			],
			path: "Sources/__APP_NAME_PASCAL__"
		),
	]
)
