#!/usr/bin/env bash
# build-plugins.sh — build the WebTransport plugin for the C# client and put
# it in packages/csharp/Plugins/<platform>, where Unity loads it.
#
#   crates/shard-client-ffi/build-plugins.sh <group>...
#
# Groups (each needs its host and toolchain):
#   macos    macOS universal dylib (arm64 + x86_64)        — a Mac
#   ios      iOS xcframework (device + simulator, dynamic)  — a Mac with Xcode
#   android  Android arm64-v8a and armeabi-v7a .so          — the NDK and cargo-ndk
#   linux    Linux x86_64 .so (glibc 2.31)                  — Linux, or Docker elsewhere
#   windows  Windows x86_64 .dll                            — Windows (MSVC), or cargo-xwin elsewhere
#   all      every group above
#
# The binaries use the `plugin` profile (Cargo.toml). Each run writes
# plugins.sha256 (source-hash.sh): CI fails when the sources change and
# the binaries were not rebuilt.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT="$(pwd)"
OUT="$ROOT/packages/csharp/Plugins"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
NAME=pylon_shard_client
build() { cargo build --profile plugin -p pylon-shard-client-ffi --target "$1"; }
lib() { echo "$TARGET_DIR/$1/plugin/$2"; }

groups=("$@")
[ "${groups[0]:-}" = all ] && groups=(macos ios android linux windows)
[ ${#groups[@]} -gt 0 ] || { echo "usage: $0 <macos|ios|android|linux|windows|all>..." >&2; exit 1; }

for group in "${groups[@]}"; do
	case "$group" in
	macos)
		export MACOSX_DEPLOYMENT_TARGET=11.0
		build aarch64-apple-darwin
		build x86_64-apple-darwin
		mkdir -p "$OUT/macOS"
		lipo -create \
			"$(lib aarch64-apple-darwin lib$NAME.dylib)" \
			"$(lib x86_64-apple-darwin lib$NAME.dylib)" \
			-output "$OUT/macOS/lib$NAME.dylib"
		install_name_tool -id "@rpath/lib$NAME.dylib" "$OUT/macOS/lib$NAME.dylib"
		;;
	ios)
		export IPHONEOS_DEPLOYMENT_TARGET=13.0
		build aarch64-apple-ios
		build aarch64-apple-ios-sim
		STAGE="$(mktemp -d)"
		for target in aarch64-apple-ios aarch64-apple-ios-sim; do
			if [[ "$target" == *sim ]]; then platform=iPhoneSimulator; else platform=iPhoneOS; fi
			fw="$STAGE/$target/$NAME.framework"
			mkdir -p "$fw"
			cp "$(lib "$target" lib$NAME.dylib)" "$fw/$NAME"
			install_name_tool -id "@rpath/$NAME.framework/$NAME" "$fw/$NAME"
			cat >"$fw/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key><string>en</string>
	<key>CFBundleExecutable</key><string>$NAME</string>
	<key>CFBundleIdentifier</key><string>dev.pylonsync.shard-client</string>
	<key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
	<key>CFBundleName</key><string>$NAME</string>
	<key>CFBundlePackageType</key><string>FMWK</string>
	<key>CFBundleShortVersionString</key><string>1.0</string>
	<key>CFBundleVersion</key><string>1</string>
	<key>CFBundleSupportedPlatforms</key><array><string>$platform</string></array>
	<key>MinimumOSVersion</key><string>13.0</string>
</dict>
</plist>
PLIST
		done
		# xcodebuild will not write over an existing xcframework: build it in
		# the stage, then sync it into place (rsync removes stale files inside).
		xcodebuild -create-xcframework \
			-framework "$STAGE/aarch64-apple-ios/$NAME.framework" \
			-framework "$STAGE/aarch64-apple-ios-sim/$NAME.framework" \
			-output "$STAGE/$NAME.xcframework" >/dev/null
		mkdir -p "$OUT/iOS/$NAME.xcframework"
		rsync -a --delete "$STAGE/$NAME.xcframework/" "$OUT/iOS/$NAME.xcframework/"
		;;
	android)
		: "${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME to the NDK}"
		cargo ndk --platform 24 -t arm64-v8a -t armeabi-v7a \
			-o "$OUT/Android" build --profile plugin -p pylon-shard-client-ffi
		# cargo ndk copies into <out>/<abi>/; strip the symbol tables it keeps.
		STRIP="$(ls -d "$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*/bin/llvm-strip | head -1)"
		for abi in arm64-v8a armeabi-v7a; do "$STRIP" --strip-unneeded "$OUT/Android/$abi/lib$NAME.so"; done
		;;
	linux)
		mkdir -p "$OUT/Linux/x86_64"
		if [ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ]; then
			build x86_64-unknown-linux-gnu
			cp "$(lib x86_64-unknown-linux-gnu lib$NAME.so)" "$OUT/Linux/x86_64/lib$NAME.so"
		else
			# Debian 11 (glibc 2.31), so the library loads on older distributions too.
			docker run --rm --platform linux/amd64 -v "$ROOT:/src" -w /src \
				-e CARGO_TARGET_DIR=/src/target/linux-plugin rust:1-bullseye \
				cargo build --profile plugin -p pylon-shard-client-ffi --target x86_64-unknown-linux-gnu
			cp "$ROOT/target/linux-plugin/x86_64-unknown-linux-gnu/plugin/lib$NAME.so" "$OUT/Linux/x86_64/lib$NAME.so"
		fi
		if command -v strip >/dev/null && [ "$(uname -s)" = Linux ]; then
			strip --strip-unneeded "$OUT/Linux/x86_64/lib$NAME.so"
		else
			docker run --rm --platform linux/amd64 -v "$OUT/Linux/x86_64:/out" rust:1-bullseye \
				strip --strip-unneeded "/out/lib$NAME.so"
		fi
		;;
	windows)
		mkdir -p "$OUT/Windows/x86_64"
		case "$(uname -s)" in
		MINGW* | MSYS* | CYGWIN*) build x86_64-pc-windows-msvc ;;
		*) cargo xwin build --profile plugin -p pylon-shard-client-ffi --target x86_64-pc-windows-msvc ;;
		esac
		cp "$(lib x86_64-pc-windows-msvc $NAME.dll)" "$OUT/Windows/x86_64/$NAME.dll"
		;;
	*)
		echo "unknown group: $group (macos, ios, android, linux, windows)" >&2
		exit 1
		;;
	esac
done

"$ROOT/crates/shard-client-ffi/source-hash.sh" >"$ROOT/crates/shard-client-ffi/plugins.sha256"
