# Runtime devshell for the cef-rs WebGPU prototype.
#
# CEF ships a prebuilt libcef.so that links against standard Linux libs
# (glib, gtk3, nss, xorg, …). On NixOS those live in /nix/store, not /usr/lib.
# This shell pulls them all in as nix packages (matching the nix-built binary's
# glibc — no FHS/glibc mismatch) and wires LD_LIBRARY_PATH + CEF_PATH so the
# binary finds libcef.so and its deps without bundling.
#
#   nix-shell shell.nix
#   # then, inside:
#   ./target/debug/afterglow-cef-webgpu --ozone-platform=wayland
#
# or one-shot:
#   nix-shell shell.nix --run "./target/debug/afterglow-cef-webgpu --ozone-platform=wayland"
{ pkgs ? import <nixpkgs> {} }:

let
  # The full set of runtime libraries libcef.so (Chromium 149) links against.
  cefRuntimeLibs = with pkgs; [
    # Core GLib stack
    glib
    # GTK3 + rendering deps (makeLibraryPath doesn't follow propagated deps,
    # so list cairo/pango/gdk-pixbuf/atk explicitly even though gtk3 pulls them)
    gtk3 at-spi2-atk atk cairo pango gdk-pixbuf
    # Crypto / cert
    nss nspr
    # Audio
    alsa-lib
    # System / IPC
    dbus expat libudev-zero
    # Printing / media (Chromium runtime)
    cups libva pipewire libgcrypt
    # Fonts
    fontconfig freetype harfbuzz
    # X11 / Wayland input (renamed from xorg.* to top-level in recent nixpkgs)
    libxkbcommon
    libsm libice
    libx11 libxcomposite libxcursor libxdamage
    libxext libxfixes libxi libxrandr libxrender
    libxtst libxscrnsaver libxcb
    # GPU / Vulkan (WebGPU via Dawn → Vulkan)
    libdrm mesa libGL libgbm vulkan-loader vulkan-validation-layers
    # C++ standard library (libstdc++.so.6)
    stdenv.cc.cc.lib
  ];
in
pkgs.mkShell {
  packages = cefRuntimeLibs ++ [
    pkgs.patchelf # handy if libcef.so needs RPATH tweaks
  ];

  shellHook = ''
    export CEF_PATH="''${CEF_PATH:-$HOME/.local/share/cef}"
    # Prefer the NixOS system libvulkan loader (/run/opengl-driver/lib) over
    # CEF's bundled one, and point it at the real GPU's Vulkan ICD. CEF bundles
    # swiftshader and its own loader, which otherwise takes over and falls back
    # to software (=> SkSurface init failures, no compositing).
    export VK_ICD_FILENAMES="''${VK_ICD_FILENAMES:-/run/opengl-driver/share/vulkan/icd.d/nvidia_icd.json:/run/opengl-driver/share/vulkan/icd.d/radeon_icd.x86_64.json:/run/opengl-driver/share/vulkan/icd.d/intel_icd.x86_64.json}"
    export LD_LIBRARY_PATH="/run/opengl-driver/lib:$CEF_PATH:${pkgs.lib.makeLibraryPath cefRuntimeLibs}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    echo "[afterglow-cef-webgpu] CEF_PATH=$CEF_PATH"
    echo "[afterglow-cef-webgpu] LD_LIBRARY_PATH set with ${toString (builtins.length cefRuntimeLibs)} lib dirs"
  '';
}
