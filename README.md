# XP Pipes

[![Build](https://github.com/vimeh/xp-pipes/actions/workflows/build.yml/badge.svg)](https://github.com/vimeh/xp-pipes/actions/workflows/build.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A recreation of Windows XP's **3D Pipes** for **Linux/Wayland and macOS**. One Rust
engine implements the recovered growth, materials, meshes, joints, camera,
multiple-pipe chasing, textures and fades. Native resolution, including Retina,
ultrawide and 8K targets, without changing XP's aspect-derived world.

**This draws pipes, not passwords.** Linux authentication belongs to your locker;
macOS activation and locking belong to the OS. No Microsoft program, symbols,
decompiled source or artwork is distributed. This is an independent project,
not affiliated with Microsoft.

> **Preview release:** Linux and native Mac rendering have been tested. The Mac
> bundle's actual sandboxed Screen Saver host, settings persistence and texture
> permissions still need manual acceptance. GPU tests and a standalone native
> preview do not establish those. This is a static reconstruction, not a claim of
> independently verified XP runtime or pixel equivalence.

## Install

Download an architecture-specific archive and `SHA256SUMS` from
[Releases](https://github.com/vimeh/xp-pipes/releases). Choose `arm64` for Apple
Silicon/ARM64 Linux, or `x86_64` for Intel Macs/x86-64 Linux.

| Platform | Requirements | Archive |
| --- | --- | --- |
| Linux | Wayland, EGL/OpenGL ES 3, glibc 2.35+ for prebuilt binaries | `xp-pipes-…-linux-{x86_64,arm64}.tar.gz` |
| macOS | macOS 14+, OpenGL 4.1 | `xp-pipes-…-macos-{arm64,x86_64}.zip` |

Compare your archive's SHA-256 with `SHA256SUMS` (`sha256sum FILE` on Linux;
`shasum -a 256 FILE` on macOS).

### Linux

Extract the archive, then:

```sh
install -Dm755 xp-pipes/bin/wlpipes "$HOME/.local/bin/wlpipes"
# Ensure ~/.local/bin is in PATH, or invoke that full path.
wlpipes --windowed --max-seconds 15
```

On Debian/Ubuntu, runtime libraries are `libwayland-client0`, `libwayland-egl1`,
`libegl1` and `libgles2`, plus a working Mesa/vendor driver. These archives do not
bundle a GPU driver. Musl-based systems should build from source or use Nix.

`--windowed` needs xdg-shell. Fullscreen `--overlay` needs layer-shell (e.g. Sway
or Niri); a standalone overlay dismisses on input and **is not a lock**. X11 is
not supported. GNOME's lack of layer-shell means the windowed preview is the
supported route there, not an automatic lock-screen background.

For an animated, authenticated lock screen, install **swaylock-plugin** separately
and configure it through your desktop/idle manager. For example:

```sh
swaylock-plugin --config /dev/null --grace 0 --color 000000 \
  --show-failed-attempts --command 'wlpipes --pipes single --joints elbow --speed-level 16'
```

Use `--command`, not `--command-each`, for independent output scenes sharing the
same process RNG/fade state. Nothing in this repository automatically changes
idle policy, keybindings or authentication configuration.

### macOS

1. Extract the ZIP and open `XP Pipes.saver` to request installation, or copy it
   into `~/Library/Screen Savers/`. Preserve any existing bundle before replacing it.
2. Choose **XP Pipes** in **System Settings → Screen Saver**, then test its preview.
3. Use its Options sheet for pipes, joints, speed, tessellation and a local texture.

The downloads are **ad-hoc signed, not Developer ID signed or notarized**. macOS
may block downloaded bundles. Only approve software you trust through macOS's
normal security UI, or build locally from source. No installer disables Gatekeeper,
removes quarantine, selects a saver, or changes password/idle policy.

OpenGL is deprecated by Apple. The port runs on the tested Apple M3, but continued
OS support is not promised. Intel archives are separately built/tested in CI;
that is not an Intel hardware GPU/Screen Saver host certification.

### Nix (both platforms)

```sh
# Linux: temporary preview or permanent command installation
nix run github:vimeh/xp-pipes -- --windowed --max-seconds 15
nix profile add github:vimeh/xp-pipes

# Either platform: build the native package
nix build github:vimeh/xp-pipes
```

On Mac the output is `result/share/screensavers/XP Pipes.saver`. Copy it to a
writable installation and sign the copy with `/usr/bin/codesign --force --sign -
'/path/to/XP Pipes.saver'`; never modify the Nix store. Keep the package GC root:
Nix builds can reference Nix runtime libraries. The release/Xcode builds use only
system libraries. [Mac build and lifecycle details](macos/README.md).

## Controls

```sh
wlpipes --windowed --pipes multiple --joints cycle --seed 42
wlpipes --windowed --pipes 2 --scene-length 2
wlpipes --windowed --texture /path/to/image.png
wlpipes --help
```

Defaults: **Single / Elbow / Solid / Speed 16 / Tessellation 0**.

- `--pipes single|multiple|1..4`: `multiple` randomly chooses 2–4 heads per scene.
  Exact counts are an extension; counts above one use the multiple/chase rules.
- `--joints elbow|ball|mixed|cycle`; `--tessellation 0..4` chooses 8/12/20/28/36 slices.
- `--speed-level 0..20`: original strict render gate; 16 means >20 ms. No catch-up
  or smooth head interpolation. Raising tessellation does not smooth chunk timing.
- `--scene-length 1..10`: optional extension multiplying the pipe-start budget,
  not an exact wall-clock duration. Defaults preserve the recovered budgets.
- `--texture PATH` accepts BMP/JPEG/PNG/TGA; `--default-texture PATH` supplies your
  own fallback artwork. No image is included or downloaded automatically.
- `--seed U32` controls process randomness; `--max-seconds 1..86400` bounds a preview.

The Mac sheet exposes the original single/multiple modes, joint modes, speed,
tessellation and texture choice; the CLI-only count/budget extensions are not
currently exposed there. See the [manual](wlpipes.1) for Linux details.

## Build and test

Rust is pinned in `rust-toolchain.toml`. On Debian/Ubuntu:

```sh
sudo apt-get install pkg-config libwayland-dev libegl1-mesa-dev libgl1-mesa-dri
cargo build --release --locked
./target/release/wlpipes --windowed --max-seconds 15
# Or: cargo install --path . --locked
```

On Mac, install Rust and Xcode/Command Line Tools, personally review/accept the
SDK license, then run `bash macos/build.sh`. The bundle appears in `dist/`.
The [bounded native preview](macos/README.md) is optional and changes no lock policy.

```sh
# Nix development environment, if preferred
nix develop
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
# Linux GPU checks, using software Mesa:
EGL_PLATFORM=surfaceless LIBGL_ALWAYS_SOFTWARE=1 \
  cargo test --locked gpu_vertex_lighting -- --ignored --nocapture
# Mac GPU checks on a machine with a native GL driver:
cargo test --locked gpu_vertex_lighting -- --ignored --nocapture
```

CI builds native archives for all four platform/architecture combinations. Linux
CI runs the production GPU checks on Mesa. Mac CI checks compilation, CPU/CLI
behavior and bundle signing; actual native GPU/host acceptance remains separate.
Tagging a matching `vX.Y.Z` publishes a preview release only after every build passes.

## Fidelity, limits and provenance

[Reconstruction notes](docs/RECONSTRUCTION.md) and
[static fixtures](tests/fixtures/README.md) distinguish recovered facts from
runtime conformance. The shared checks include 144 animation cases through three
resets and 780 mesh/material GPU cases, plus actual allocations through 8K.
Framebuffer storage is budgeted, not unlimited: 128 MiB model/upload budget,
512 MiB aggregate estimated GPU storage, 16384 pixels per dimension and 64 MP.
Driver window buffers are additional. Expected XP margins/clipping are preserved.

Extracted from [vimeh/nixos-config](https://github.com/vimeh/nixos-config) at
`b1e5def4963efea4bbbe1e5619a470a49bbd3cd2`; no unrelated system configuration is
included. The native Wayland infrastructure originated in
[flacyak/wlpipes](https://github.com/flacyak/wlpipes).
[MIT license](LICENSE), with original attribution retained in [NOTICE](NOTICE).
