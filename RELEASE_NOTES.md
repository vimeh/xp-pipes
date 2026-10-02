First standalone preview release of XP Pipes, extracted from `vimeh/nixos-config`.

## Downloads

- Linux/Wayland: `linux-x86_64.tar.gz` or `linux-arm64.tar.gz` (glibc 2.35+, EGL/GLES 3).
- macOS 14+: `macos-arm64.zip` for Apple Silicon or `macos-x86_64.zip` for Intel.
- `SHA256SUMS` covers all four archives.

See the [README](https://github.com/vimeh/xp-pipes#install) for installation, source
builds, Nix and separate Linux locking integration. No installer modifies your
selected saver, idle policy or authentication settings.

## Important limitations

**macOS is experimental:** native Apple M3 GPU/Retina/lifecycle checks pass, but
Apple's actual sandboxed Screen Saver host, settings persistence and texture
permissions still need manual acceptance. Downloads are ad-hoc signed, **not
Developer ID signed or notarized**; macOS may block downloaded bundles. Build
locally from reviewed source or use the normal OS approval UI only if you trust
the software. No security-bypass commands are provided.

Linux windowed previews need xdg-shell; fullscreen overlays need layer-shell.
An overlay is not a screen lock. Use a real locker such as swaylock-plugin/PAM
for authentication. X11 is not supported.

The engine reconstructs XP behavior from static evidence; independent XP runtime/
pixel equivalence is not certified. Microsoft executables, symbols, decompiled
source and artwork are not distributed. Growth smoothing is not included.
