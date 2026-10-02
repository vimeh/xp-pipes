{
  lib,
  stdenv,
  buildPackages,
  rustPlatform,
  pkg-config,
  autoPatchelfHook,
  wayland,
  libglvnd,
  mesa,
}:

rustPlatform.buildRustPackage {
  pname = "wlpipes";
  version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;

  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./Cargo.toml
      ./Cargo.lock
      ./src
      ./tests
      ./LICENSE
      ./NOTICE
      ./README.md
      ./wlpipes.1
    ];
  };
  cargoLock.lockFile = ./Cargo.lock;
  nativeBuildInputs = [
    buildPackages.clippy
    buildPackages.rustfmt
    pkg-config
    autoPatchelfHook
  ];
  buildInputs = [
    wayland
    libglvnd
    stdenv.cc.cc.lib
  ];
  nativeCheckInputs = [ mesa ];
  preBuild = builtins.readFile ./rust-quality.sh;
  env.CARGO_BUILD_TARGET = stdenv.hostPlatform.rust.rustcTarget;
  # CLI tests spawn the binary before autoPatchelf's installation fixup.
  preCheck = ''
    export LD_LIBRARY_PATH="${
      lib.makeLibraryPath [
        wayland
        libglvnd
      ]
    }"
  '';

  # Test the actual vertex outputs and raster state on a deterministic software
  # GLES driver, without requiring a compositor or a hardware render node.
  postCheck = ''
    EGL_PLATFORM=surfaceless LIBGL_ALWAYS_SOFTWARE=1 \
      __EGL_VENDOR_LIBRARY_FILENAMES=${mesa}/share/glvnd/egl_vendor.d/50_mesa.json \
      cargo test --offline --locked gpu_vertex_lighting -- --ignored --nocapture
  '';

  postInstall = ''
    install -Dm644 LICENSE $out/share/licenses/wlpipes/LICENSE
    install -Dm644 NOTICE $out/share/licenses/wlpipes/NOTICE
    install -Dm644 README.md $out/share/doc/wlpipes/README.md
    install -Dm644 wlpipes.1 $out/share/man/man1/wlpipes.1
  '';

  meta = {
    description = "Windows XP 3D Pipes recreation for Wayland, implemented in Rust";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = "wlpipes";
  };
}
