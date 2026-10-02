#!/usr/bin/env bash
# Native build only; never installs/selects the saver or changes lock policy.
set -euo pipefail
cd "$(dirname "$0")/.."
output="${1:-$PWD/dist}"
mkdir -p "$output"
output="$(cd "$output" && pwd)"
bundle="$output/XP Pipes.saver"
export MACOSX_DEPLOYMENT_TARGET=14.0
export SDKROOT="${SDKROOT:-$(xcrun --sdk macosx --show-sdk-path)}"
cargo rustc --locked --release --lib --crate-type staticlib
archive="${CARGO_TARGET_DIR:-target}/${CARGO_BUILD_TARGET:+$CARGO_BUILD_TARGET/}release/libwlpipes.a"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp macos/Info.plist "$bundle/Contents/Info.plist"
cp LICENSE NOTICE "$bundle/Contents/Resources/"
"${CC:-clang}" -isysroot "$SDKROOT" -mmacosx-version-min="$MACOSX_DEPLOYMENT_TARGET" -fobjc-arc -O2 -Wall -Wextra -Werror \
    -Wno-deprecated-declarations -bundle macos/PipesView.m "$archive" \
    -framework ScreenSaver -framework AppKit -framework OpenGL -liconv -lresolv -lc++ \
    -o "$bundle/Contents/MacOS/XPPipes"
"${CC:-clang}" -isysroot "$SDKROOT" -mmacosx-version-min="$MACOSX_DEPLOYMENT_TARGET" -fobjc-arc -O2 -Wall -Wextra -Werror \
    -Wno-deprecated-declarations macos/Preview.m -framework ScreenSaver -framework AppKit -framework OpenGL \
    -o "$output/xp-pipes-preview"
# Nix produces an unsigned bundle; sign the writable installation, not the store.
if [[ "${WLPIPES_SIGN:-1}" != 0 ]]; then
    /usr/bin/codesign --force --sign - "$bundle"
    /usr/bin/codesign --verify --strict "$bundle"
fi
printf 'Built %s\n' "$bundle"
