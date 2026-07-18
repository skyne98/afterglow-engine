let
  # Immutable nixpkgs revision used for the recorded fox-laptop measurements.
  pkgs = import (builtins.fetchTarball
    "https://github.com/NixOS/nixpkgs/archive/aa290c9891fa.tar.gz") {};
in
pkgs.mkShell {
  packages = with pkgs; [
    assimp
    bun
    cmake
    curl
    emscripten
    git
    python3
    unzip
  ];
}
