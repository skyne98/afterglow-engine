{
  description = "Afterglow Engine — Bevy-based game engine dev shell";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

        wasmPkgs = with pkgs; [
          binaryen  # wasm-opt
          wasm-bindgen-cli
          python3
        ];

        bevyLibs = with pkgs; [
          libx11
          libxcursor
          libxrandr
          libxi
          libxkbcommon
          wayland
          alsa-lib
          udev
          vulkan-loader
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            rustToolchain
            clang
            mold
            pkg-config
            just
          ] ++ wasmPkgs;

          buildInputs = bevyLibs;

          shellHook = ''
            export LD_LIBRARY_PATH="${with pkgs; lib.makeLibraryPath ([
              vulkan-loader
              alsa-lib
              udev
              libxkbcommon
              wayland
              libx11
              libxcursor
              libxrandr
              libxi
            ])}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
            export RUST_SRC_PATH="${rustToolchain}/lib/rustlib/src/rust/library"
            echo "🔧 afterglow-engine dev shell ready"
          '';
        };
      });
}
