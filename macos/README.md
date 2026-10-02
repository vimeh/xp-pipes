# Native macOS screen saver

`XP Pipes.saver` wraps the shared Rust engine in `ScreenSaverView`. macOS owns
activation and authentication. The bundle never selects itself or modifies idle,
password or Gatekeeper settings.

## Build from this repository

Requires macOS 14+, Rust (see `../rust-toolchain.toml`), and an installed Xcode/SDK
whose license you have personally reviewed and accepted.

```sh
bash macos/build.sh /tmp/xp-pipes-build
/tmp/xp-pipes-build/xp-pipes-preview '/tmp/xp-pipes-build/XP Pipes.saver'
```

The helper opens two windows for eight seconds, constructs the settings sheet,
resizes landscape→portrait, stops/restarts one view and checks Retina dimensions.
It does not select a saver or lock. It is **not Apple's sandboxed legacy saver host**.

If custom Cargo linker flags select an LLD version too old to read your SDK's
architecture names (observed with SDK 27), use Apple's linker for this command:

```sh
RUSTFLAGS='-C linker=/usr/bin/clang' CC=/usr/bin/clang \
  bash macos/build.sh /tmp/xp-pipes-build
```

This does not edit Cargo's configuration. Nix's pinned SDK is another option:
`nix develop` and `nix build` from the repository root. Nix emits an unsigned bundle
at `result/share/screensavers/XP Pipes.saver`; copy and sign the writable copy,
and keep the package GC root because it may reference Nix runtime libraries.

## Install deliberately

Quit previews first. Preserve an existing bundle before replacing it. For a new,
locally built installation:

```sh
source='/tmp/xp-pipes-build/XP Pipes.saver'
destination="$HOME/Library/Screen Savers/XP Pipes.saver"
mkdir -p "$(dirname "$destination")"
if [ ! -e "$destination" ] && [ ! -L "$destination" ]; then
  ditto "$source" "$destination" &&
    /usr/bin/codesign --force --sign - "$destination" &&
    /usr/bin/codesign --verify --strict "$destination"
else
  printf 'Preserve the existing bundle first: %s\n' "$destination"
fi
```

Select XP Pipes in System Settings → Screen Saver and verify its preview when
you are ready. Selection is deliberately manual. Ad-hoc signing is **not Developer
ID signing or notarization**. Browser-downloaded release bundles can be blocked
by macOS. Do not disable security protections; approve only software you trust
through the OS's normal UI, or build from reviewed source locally.

## Settings and lifecycle

- The native sheet exposes Single/Multiple, Elbow/Ball/Mixed/Cycle, Speed 0–20,
  Tessellation 0–4 and a texture picker. Defaults are Single/Elbow/Solid/16/0.
  Preferences use the module domain `dev.vinay.XPPipes`.
- Textures use security-scoped bookmarks while decoding. Picker/bookmark persistence
  and access still need acceptance in Apple's actual sandboxed saver host.
- Best-resolution OpenGL surfaces and backing-pixel sizes provide Retina rendering;
  logical aspect still controls XP's world. Temporarily unavailable drawables are skipped.
- Host callbacks drive rendering. Stop retires resources/context and stops work.
  Restart or native context replacement starts a new scene. Ordinary framebuffer
  resizing replays history without advancing RNG.
- Live same-main-thread views share process randomness, fades and resource budgets.
  Separate host processes necessarily have separate state. Each view owns its context.
- Rust unwinds are contained at the C interface. Rendering failure cannot unlock
  the session or bypass OS authentication.

## Validation scope

Native Apple M3 testing passed Clippy, CPU/CLI fixtures, the 144-case animation
matrix, the 780-case GPU matrix, actual targets through 8K and Retina/resize/
start-stop/two-view harness checks. CGL tests use FBOs: the legacy pbuffer path
was unsupported on the tested Apple Silicon driver. These are reconstruction
checks, not independent XP runtime/pixel certification.

The public CI's Mac jobs do not certify a native GPU or Apple's saver host. Actual
host loading, settings persistence, texture permissions, physical display hotplug,
sleep/wake and live lock/unlock remain explicit manual gates.

OpenGL is deprecated. Native feasibility is established on the tested M3, not
promised for future OS versions. No second simulation or speculative Metal backend
is included.
