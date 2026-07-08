#!/bin/sh
# Runner for the cef-rs WebGPU prototype inside an FHS env (steam-run).
# On NixOS, CEF's prebuilt libcef.so expects standard FHS libs (glib, gtk, nss…)
# which steam-run provides. $CEF_PATH supplies libcef.so + resources.
export CEF_PATH="${CEF_PATH:-$HOME/.local/share/cef}"
export LD_LIBRARY_PATH="$CEF_PATH:/lib:/usr/lib:/usr/lib64${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
HERE="$(cd "$(dirname "$0")" && pwd)"
exec "$HERE/target/debug/afterglow-cef-webgpu" --ozone-platform=wayland "$@"
