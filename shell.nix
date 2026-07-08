# Root devShell for the afterglow-engine workspace (NixOS).
#
# ALWAYS build and run through this shell so the environment (especially
# CEF_PATH) is consistent — otherwise cef-dll-sys re-downloads/rebuilds CEF
# from scratch each time.
#
#   nix-shell shell.nix --run "cargo build --example minimal"
#   nix-shell shell.nix --run "./target/debug/examples/minimal --ozone-platform=x11"
#
# Rust toolchain is inherited from the system (consistent across runs => no
# rustc-change rebuilds). This shell adds CEF's runtime libs + the real Vulkan
# ICD wiring that the prebuilt libcef.so needs on NixOS.
{ pkgs ? import <nixpkgs> {} }:

let
  # The standard Linux libs CEF's prebuilt libcef.so links against (NixOS keeps
  # these in /nix/store, not /usr/lib). makeLibraryPath doesn't follow propagated
  # deps, so cairo/pango/gdk-pixbuf/atk are listed explicitly.
  cefRuntimeLibs = with pkgs; [
    glib gtk3 at-spi2-atk atk cairo pango gdk-pixbuf
    nss nspr alsa-lib dbus expat libudev-zero
    fontconfig freetype harfbuzz libxkbcommon
    libsm libice libx11 libxcomposite libxcursor libxdamage
    libxext libxfixes libxi libxrandr libxrender libxtst libxscrnsaver libxcb
    libdrm mesa libGL libgbm vulkan-loader vulkan-validation-layers
    cups libva pipewire libgcrypt
    stdenv.cc.cc.lib
  ];
in
pkgs.mkShell {
  packages = cefRuntimeLibs ++ [ pkgs.patchelf pkgs.mold pkgs.clang ];

  shellHook = ''
    # CEF resources (libcef.so, locales, *.pak) live in the workspace target
    # after the first build. Pinning CEF_PATH here makes cef-dll-sys reuse them
    # instead of re-downloading on every build.
    export CEF_PATH="''${CEF_PATH:-$PWD/target/debug}"
    # Prefer the NixOS system libvulkan loader over CEF's bundled swiftshader,
    # and point it at the real GPU ICD (avoids software fallback / SkSurface
    # init failures).
    export VK_ICD_FILENAMES="''${VK_ICD_FILENAMES:-/run/opengl-driver/share/vulkan/icd.d/nvidia_icd.json:/run/opengl-driver/share/vulkan/icd.d/radeon_icd.x86_64.json:/run/opengl-driver/share/vulkan/icd.d/intel_icd.x86_64.json}"
    export LD_LIBRARY_PATH="/run/opengl-driver/lib:$CEF_PATH:${pkgs.lib.makeLibraryPath cefRuntimeLibs}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    echo "[afterglow-engine] devShell ready  CEF_PATH=$CEF_PATH"
    echo "[afterglow-engine] tip: nix-shell shell.nix --run \"cargo ...\""
  '';
}
