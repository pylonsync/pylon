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
        // CSQLite intentionally does NOT use pkgConfig: on macOS that would
        // point the linker at Homebrew's libsqlite3 (/opt/homebrew/...), which
        // bakes a brew-only install path into shipping binaries — they crash
        // at launch on every machine that doesn't have that exact brew
        // formula. The module.modulemap's `link "sqlite3"` is enough: the
        // linker resolves -lsqlite3 via the macOS SDK to /usr/lib/libsqlite3.dylib,
        // which is the ABI-stable system sqlite that ships with every macOS.
        .systemLibrary(
            name: "CSQLite",
            path: "Sources/CSQLite"
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
