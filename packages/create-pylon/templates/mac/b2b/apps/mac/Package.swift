// swift-tools-version:5.9
import PackageDescription

let package = Package(
	name: "__APP_NAME_PASCAL__",
	platforms: [
		.macOS(.v13),
	],
	dependencies: [
		.package(url: "https://github.com/pylonsync/pylon-swift.git", from: "0.3.0"),
	],
	targets: [
		.executableTarget(
			name: "__APP_NAME_PASCAL__",
			dependencies: [
				.product(name: "PylonClient", package: "pylon-swift"),
				.product(name: "PylonSwiftUI", package: "pylon-swift"),
			],
			path: "Sources/__APP_NAME_PASCAL__"
		),
	]
)
