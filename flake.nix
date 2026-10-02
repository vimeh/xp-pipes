{
  description = "XP Pipes — Windows XP 3D Pipes for Wayland and macOS";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/4975466d324710c576dc11ad614684e6bd8cad8e";

  outputs =
    { nixpkgs, ... }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
        "x86_64-darwin"
      ];
      forAll = nixpkgs.lib.genAttrs systems;
      pkgsFor = system: import nixpkgs { inherit system; };
      packageFor =
        pkgs:
        pkgs.callPackage (if pkgs.stdenv.hostPlatform.isDarwin then ./darwin.nix else ./default.nix) { };
    in
    {
      packages = forAll (
        system:
        let
          package = packageFor (pkgsFor system);
        in
        {
          default = package;
          wlpipes = package;
        }
      );
      checks = forAll (system: {
        wlpipes = packageFor (pkgsFor system);
      });
      devShells = forAll (
        system:
        let
          pkgs = pkgsFor system;
          shell = pkgs.mkShell (
            {
              inputsFrom = [ (packageFor pkgs) ];
              nativeBuildInputs = with pkgs; [
                cargo
                rustc
                rustfmt
                clippy
                rust-analyzer
              ];
            }
            // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
              LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
                pkgs.wayland
                pkgs.libglvnd
              ];
            }
          );
        in
        {
          default = shell;
          wlpipes = shell;
        }
      );
    };
}
