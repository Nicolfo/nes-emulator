#!/bin/sh
# Installs the emulator, its launcher entry and icon into a user-local prefix
# (no root needed). After this, "NES Emulator" appears in your app launcher
# with its icon, and double-clicking .nes files can be routed to it.
#
# Override the location with PREFIX, e.g.  PREFIX=/usr/local sudo ./install.sh
set -e

PREFIX="${PREFIX:-$HOME/.local}"
HERE="$(cd "$(dirname "$0")" && pwd)"

install -Dm755 "$HERE/nes-emulator"          "$PREFIX/bin/nes-emulator"
install -Dm644 "$HERE/nes-emulator.png"      "$PREFIX/share/icons/hicolor/256x256/apps/nes-emulator.png"
install -Dm644 "$HERE/nes-emulator.desktop"  "$PREFIX/share/applications/nes-emulator.desktop"

# Refresh the desktop/icon caches if the tools are present (best-effort).
update-desktop-database "$PREFIX/share/applications" 2>/dev/null || true
gtk-update-icon-cache -f "$PREFIX/share/icons/hicolor" 2>/dev/null || true

echo "Installed to $PREFIX."
case ":$PATH:" in
	*":$PREFIX/bin:"*) ;;
	*) echo "Note: add $PREFIX/bin to your PATH to launch 'nes-emulator' from a terminal." ;;
esac
