// swift-tools-version:5.9
import PackageDescription

// Root-level Package.swift so SwiftPM consumers can pull the Swift SDK
// directly from this monorepo:
//
//     .package(url: "https://github.com/pylonsync/pylon.git", from: "0.3.0")
//
// SwiftPM looks for Package.swift at the repo root by default — without
// this file, `swift package resolve` fails with "the package manifest
// at '/Package.swift' cannot be accessed". The actual sources live under
// packages/swift/, and this manifest re-roots every target's `path`
// there. Tests are intentionally skipped: external consumers don't run
// our SDK tests, and including them here would force the test
// dependency tree (XCTest helpers etc.) into every consumer's build.
//
// If you're working on the SDK itself, use `packages/swift/Package.swift`
// directly — it's the same target tree without the path prefix.
let package = Package(
    name: "pylon",
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
            path: "packages/swift/Sources/PylonClient"
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
            path: "packages/swift/Sources/CSQLite"
        ),
        .target(
            name: "PylonSync",
            dependencies: [
                "PylonClient",
                "CSQLite",
                .product(name: "Loro", package: "loro-swift"),
            ],
            path: "packages/swift/Sources/PylonSync"
        ),
        .target(
            name: "PylonRealtime",
            dependencies: ["PylonClient"],
            path: "packages/swift/Sources/PylonRealtime"
        ),
        .target(
            name: "PylonSwiftUI",
            dependencies: ["PylonSync"],
            path: "packages/swift/Sources/PylonSwiftUI"
        ),
    ]
)
