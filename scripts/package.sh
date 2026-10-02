#!/usr/bin/env bash
# Build a native release archive, without installing or activating anything.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p dist
version=$(cargo metadata --locked --no-deps --format-version 1 | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "wlpipes"))')
if [[ ${GITHUB_REF_TYPE:-} == tag && ${GITHUB_REF_NAME:-} != "v$version" ]]; then
    printf 'Tag must match Cargo version v%s\n' "$version" >&2
    exit 1
fi
case $(uname -m) in
    x86_64) arch=x86_64 ;;
    arm64|aarch64) arch=arm64 ;;
    *) printf 'Unsupported release architecture\n' >&2; exit 1 ;;
esac
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
case $(uname -s) in
    Linux)
        cargo build --locked --release
        binary="${CARGO_TARGET_DIR:-target}/${CARGO_BUILD_TARGET:+$CARGO_BUILD_TARGET/}release/wlpipes"
        install -Dm755 "$binary" "$stage/xp-pipes/bin/wlpipes"
        install -Dm644 wlpipes.1 "$stage/xp-pipes/share/man/man1/wlpipes.1"
        cp LICENSE NOTICE README.md "$stage/xp-pipes/"
        tar -czf "dist/xp-pipes-$version-linux-$arch.tar.gz" -C "$stage" xp-pipes
        ;;
    Darwin)
        bash macos/build.sh "$stage"
        /usr/bin/codesign --verify --strict "$stage/XP Pipes.saver"
        actual=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$stage/XP Pipes.saver/Contents/Info.plist")
        [[ $actual == "$version" ]]
        ditto -c -k --sequesterRsrc --keepParent "$stage/XP Pipes.saver" "dist/xp-pipes-$version-macos-$arch.zip"
        ;;
    *) printf 'Only Linux and macOS are supported\n' >&2; exit 1 ;;
esac
