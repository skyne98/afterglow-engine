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
    libdrm mesa libGL libgbm vulkan-loader
    cups libva pipewire libgcrypt
    stdenv.cc.cc.lib
  ];
in
pkgs.mkShell {
  packages = cefRuntimeLibs ++ [ pkgs.patchelf pkgs.mold pkgs.clang pkgs.bun ];

  shellHook = ''
    # CEF resources (libcef.so, locales, *.pak) live in the workspace target
    # after the first build. Pinning CEF_PATH here makes cef-dll-sys reuse them
    # instead of re-downloading on every build.
    export CEF_PATH="''${CEF_PATH:-$PWD/target/debug}"
    # CEF bundles libvulkan.so.1 + SwiftShader (software Vulkan, without the
    # surface extensions CEF needs). Select one *coherent* real Vulkan stack
    # ahead of CEF's directory. On NixOS /run/opengl-driver is the authoritative
    # host stack. On FHS hosts the default is Nix's loader + Mesa ICD: Fedora 44
    # Mesa 26.1.4 RADV crashes CEF 149's GPU process with SIGFPE in
    # radv_clear_dcc_comp_to_single; Nix Mesa 25.3.4 is validated on fox-laptop
    # (Radeon 680M). Do not use the host stack there unless diagnosing it.
    if [ -d /run/opengl-driver/lib ]; then
      graphicsLibDirs=/run/opengl-driver/lib
      export VK_ICD_FILENAMES="''${VK_ICD_FILENAMES:-/run/opengl-driver/share/vulkan/icd.d/nvidia_icd.json:/run/opengl-driver/share/vulkan/icd.d/radeon_icd.x86_64.json:/run/opengl-driver/share/vulkan/icd.d/intel_icd.x86_64.json}"
    elif [ "''${AFTERGLOW_VULKAN_STACK:-nix}" = host ] && [ -e /usr/lib64/libvulkan.so.1 ]; then
      # Escape hatch for diagnosing a distro driver; not validated on fox-laptop.
      graphicsLibDirs=/usr/lib64
    elif [ "''${AFTERGLOW_VULKAN_STACK:-nix}" = host ] && [ -e /usr/lib/libvulkan.so.1 ]; then
      graphicsLibDirs=/usr/lib
    else
      graphicsLibDirs="${pkgs.vulkan-loader}/lib:${pkgs.mesa}/lib:${pkgs.libdrm}/lib:${pkgs.libGL}/lib:${pkgs.libgbm}/lib"
      export VK_ICD_FILENAMES="''${VK_ICD_FILENAMES:-${pkgs.mesa}/share/vulkan/icd.d/radeon_icd.x86_64.json:${pkgs.mesa}/share/vulkan/icd.d/intel_icd.x86_64.json}"
    fi
    export LD_LIBRARY_PATH="$graphicsLibDirs:$CEF_PATH:${pkgs.lib.makeLibraryPath cefRuntimeLibs}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    # X11/XWayland display (the app runs --ozone-platform=x11). Xwayland is
    # typically at :0 on a Wayland session; fall back to it if unset.
    export DISPLAY="''${DISPLAY:-:0}"
    echo "[afterglow-engine] devShell ready  CEF_PATH=$CEF_PATH  DISPLAY=$DISPLAY"
    echo "[afterglow-engine] tip: nix-shell shell.nix --run \"cargo ...\""
  '';
}
