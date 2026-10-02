{
  lib,
  stdenv,
  rustPlatform,
  buildPackages,
  libiconv,
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
      ./macos
      ./LICENSE
      ./NOTICE
      ./README.md
    ];
  };
  cargoLock.lockFile = ./Cargo.lock;
  nativeBuildInputs = [
    buildPackages.clippy
    buildPackages.rustfmt
  ];
  buildInputs = [ libiconv ];
  env.CARGO_BUILD_TARGET = stdenv.hostPlatform.rust.rustcTarget;
  preBuild = builtins.readFile ./rust-quality.sh;
  buildPhase = ''
    runHook preBuild
    WLPIPES_SIGN=0 bash macos/build.sh "$TMPDIR/bundle"
    runHook postBuild
  '';
  # GPU tests require a logged-in, unsandboxed native session; run explicitly on Lotus.
  installPhase = ''
    runHook preInstall
    mkdir -p "$out/share/screensavers" "$out/bin" "$out/share/licenses/wlpipes"
    cp -R "$TMPDIR/bundle/XP Pipes.saver" "$out/share/screensavers/"
    cp "$TMPDIR/bundle/xp-pipes-preview" "$out/bin/"
    cp LICENSE NOTICE "$out/share/licenses/wlpipes/"
    runHook postInstall
  '';
  # Sign the copied bundle after installation, never mutate the Nix store.
  dontStrip = true;
  meta = {
    description = "Native macOS screen saver using the shared XP Pipes recreation";
    license = lib.licenses.mit;
    platforms = lib.platforms.darwin;
  };
}
