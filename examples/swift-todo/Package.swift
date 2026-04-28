// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "PylonTodoApp",
    platforms: [
        .iOS(.v16),
        .macOS(.v14),
    ],
    products: [
        .executable(name: "PylonTodoApp", targets: ["PylonTodoApp"]),
    ],
    dependencies: [
        // Local path during development; switch to the published URL after release:
        // .package(url: "https://github.com/pylonsync/pylon-swift.git", from: "0.3.0"),
        .package(path: "../../packages/swift"),
    ],
    targets: [
        .executableTarget(
            name: "PylonTodoApp",
            dependencies: [
                .product(name: "PylonClient", package: "swift"),
                .product(name: "PylonSync",   package: "swift"),
                .product(name: "PylonSwiftUI",package: "swift"),
            ],
            path: "PylonTodoApp"
        ),
    ]
)
