# Root devShell for the afterglow-engine workspace (NixOS).
#
# The native host is `afterglow-shell` (rusty_v8 + deno_webgpu/wgpu-core +
# winit + Blitz + Vello). It needs the real Vulkan loader + ICD on the host.
#
#   nix-shell shell.nix --run "cargo build -p afterglow-shell"
#   nix-shell shell.nix --run "cargo run -p afterglow-shell"
#
# Rust toolchain is inherited from the system (consistent across runs => no
# rustc-change rebuilds). This shell provides the Vulkan/graphics stack that
# wgpu-core surfaces through, plus native libs the shell links against.
{ pkgs ? import <nixpkgs> {} }:

let
  # Graphics + native libs the shell's wgpu/winit surface and Blitz text
  # layout link against (NixOS keeps these in /nix/store, not /usr/lib).
  runtimeLibs = with pkgs; [
    glib gtk3 at-spi2-atk atk cairo pango gdk-pixbuf
    expat libudev-zero
    fontconfig freetype harfbuzz libxkbcommon
    wayland
    libsm libice libx11 libxcomposite libxcursor libxdamage
    libxext libxfixes libxi libxrandr libxrender libxtst libxcb
    libdrm mesa libGL libgbm vulkan-loader
    stdenv.cc.cc.lib
  ];
in
pkgs.mkShell {
  packages = runtimeLibs ++ [
    pkgs.patchelf pkgs.mold pkgs.clang pkgs.bun pkgs.caddy
    # Stylo generates its property tables while building afterglow-shell.
    pkgs.python3
  ];

  shellHook = ''
    # Select one *coherent* real Vulkan stack. On NixOS /run/opengl-driver is
    # the authoritative host stack. On FHS hosts fall back to Nix's loader +
    # Mesa ICD. (The prior CEF-specific Mesa 26.1.4 RADV SIGFPE note applied
    # to CEF's bundled libcef.so; the shell uses the system wgpu stack.)
    if [ -d /run/opengl-driver/lib ]; then
      graphicsLibDirs=/run/opengl-driver/lib
      export VK_ICD_FILENAMES="''${VK_ICD_FILENAMES:-/run/opengl-driver/share/vulkan/icd.d/nvidia_icd.json:/run/opengl-driver/share/vulkan/icd.d/radeon_icd.x86_64.json:/run/opengl-driver/share/vulkan/icd.d/intel_icd.x86_64.json}"
    elif [ "''${AFTERGLOW_VULKAN_STACK:-nix}" = host ] && [ -e /usr/lib64/libvulkan.so.1 ]; then
      graphicsLibDirs=/usr/lib64
    elif [ "''${AFTERGLOW_VULKAN_STACK:-nix}" = host ] && [ -e /usr/lib/libvulkan.so.1 ]; then
      graphicsLibDirs=/usr/lib
    else
      graphicsLibDirs="${pkgs.vulkan-loader}/lib:${pkgs.mesa}/lib:${pkgs.libdrm}/lib:${pkgs.libGL}/lib:${pkgs.libgbm}/lib"
      export VK_ICD_FILENAMES="''${VK_ICD_FILENAMES:-${pkgs.mesa}/share/vulkan/icd.d/radeon_icd.x86_64.json:${pkgs.mesa}/share/vulkan/icd.d/intel_icd.x86_64.json}"
    fi
    export LD_LIBRARY_PATH="$graphicsLibDirs:${pkgs.lib.makeLibraryPath runtimeLibs}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    # X11/XWayland display. Xwayland is typically at :0 on a Wayland session.
    export DISPLAY="''${DISPLAY:-:0}"
    echo "[afterglow-engine] devShell ready  DISPLAY=$DISPLAY"
    echo "[afterglow-engine] tip: nix-shell shell.nix --run \"cargo ...\""
  '';
}
