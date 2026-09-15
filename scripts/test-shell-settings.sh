#!/usr/bin/env bash
# Run with: nix-shell shell.nix --run 'bash scripts/test-shell-settings.sh'
set -euo pipefail
# Use memory-only settings to prevent changes to the desktop settings.
export GSETTINGS_BACKEND=memory
gsettings get org.gtk.Settings.FileChooser show-hidden >/dev/null
gsettings get org.gnome.desktop.interface font-name >/dev/null
printf '%s\n' 'Native GTK schema checks passed.'
