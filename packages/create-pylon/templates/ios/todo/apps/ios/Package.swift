// swift-tools-version:5.9
import PackageDescription

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
