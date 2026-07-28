#!/usr/bin/env bash
#
# Install a desktop launcher for Folio so it appears in the application menu /
# search on any freedesktop desktop (GNOME, KDE, …) — Ubuntu, Fedora, etc.
#
# Per-user install (no sudo). Build the release binary first:
#     cargo build --release            # inside src-tauri/
# then run this script:
#     scripts/install-desktop.sh
#
# Paths are resolved relative to this script, so nothing needs editing.

set -euo pipefail

# Repo root = parent of this script's directory.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

BIN="$ROOT/src-tauri/target/release/folio"
ICONS="$ROOT/src-tauri/icons"

if [[ ! -x "$BIN" ]]; then
    echo "error: $BIN not found." >&2
    echo "Build it first:  (cd src-tauri && cargo build --release)" >&2
    exit 1
fi

HIC="$HOME/.local/share/icons/hicolor"
APPS="$HOME/.local/share/applications"

# Install the icon into the hicolor theme under the name "folio".
mkdir -p "$HIC/32x32/apps" "$HIC/128x128/apps" "$HIC/256x256/apps" "$APPS"
cp "$ICONS/32x32.png"      "$HIC/32x32/apps/folio.png"
cp "$ICONS/128x128.png"    "$HIC/128x128/apps/folio.png"
cp "$ICONS/128x128@2x.png" "$HIC/256x256/apps/folio.png"

# Write the launcher entry. Exec points at the build output, so rebuilding
# updates the launched binary automatically.
cat > "$APPS/folio.desktop" <<EOF
[Desktop Entry]
Type=Application
Version=1.0
Name=Folio
GenericName=PDF Reader
Comment=PDF reader and library manager
Exec=$BIN
Icon=folio
Terminal=false
Categories=Office;Viewer;
StartupWMClass=folio
Keywords=PDF;reader;library;documents;
EOF

# Refresh the desktop/icon caches (best-effort; harmless if the tools are absent).
update-desktop-database "$APPS" 2>/dev/null || true
gtk-update-icon-cache -f -t "$HIC" 2>/dev/null || true

echo "Installed. Search 'Folio' in your application menu."
echo "(If it doesn't appear right away, log out and back in.)"
