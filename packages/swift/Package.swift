// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "pylon-swift",
    platforms: [
        .iOS(.v16),
        .macOS(.v13),
        .tvOS(.v16),
        .watchOS(.v9),
    ],
    products: [
        .library(name: "PylonClient", targets: ["PylonClient"]),
        .library(name: "PylonSync", targets: ["PylonSync"]),
        .library(name: "PylonRealtime", targets: ["PylonRealtime"]),
        .library(name: "PylonSwiftUI", targets: ["PylonSwiftUI"]),
    ],
    dependencies: [
        .package(url: "https://github.com/loro-dev/loro-swift.git", from: "1.10.3"),
    ],
    targets: [
        .target(
            name: "PylonClient",
            dependencies: [],
            path: "Sources/PylonClient"
        ),
        .systemLibrary(
            name: "CSQLite",
            path: "Sources/CSQLite",
            pkgConfig: "sqlite3",
            providers: [
                .apt(["libsqlite3-dev"]),
                .brew(["sqlite3"]),
            ]
        ),
        .target(
            name: "PylonSync",
            dependencies: [
                "PylonClient",
                "CSQLite",
                .product(name: "Loro", package: "loro-swift"),
            ],
            path: "Sources/PylonSync"
        ),
        .target(
            name: "PylonRealtime",
            dependencies: ["PylonClient"],
            path: "Sources/PylonRealtime"
        ),
        .target(
            name: "PylonSwiftUI",
            dependencies: ["PylonSync"],
            path: "Sources/PylonSwiftUI"
        ),
        .testTarget(
            name: "PylonClientTests",
            dependencies: ["PylonClient"],
            path: "Tests/PylonClientTests"
        ),
        .testTarget(
            name: "PylonSyncTests",
            dependencies: ["PylonSync"],
            path: "Tests/PylonSyncTests"
        ),
    ]
)
