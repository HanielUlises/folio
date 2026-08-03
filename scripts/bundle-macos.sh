#!/usr/bin/env bash
# Build Folio.app (and optionally a drag-to-install .dmg) for macOS.
#
#   scripts/bundle-macos.sh [--arch universal|x86_64|arm64] [--features drive] [--dmg]
#
# The default `universal` build runs natively on Apple Silicon and Intel, so one
# bundle covers every Mac able to run macOS 11 Big Sur (2020) or later.
#
# Signing
# -------
# With no configuration the bundle is signed ad-hoc, which is enough to run on
# the machine that built it but *not* enough for distribution: macOS blocks a
# downloaded copy. To produce something a non-technical user can just open, set
#
#   FOLIO_SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)"
#
# and, to notarise as well (requires an Apple Developer account):
#
#   FOLIO_NOTARIZE_APPLE_ID / FOLIO_NOTARIZE_TEAM_ID / FOLIO_NOTARIZE_PASSWORD
#
# Must be run on macOS: it needs the Xcode command line tools.

set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "error: this script must be run on macOS." >&2
    exit 1
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE="$ROOT/src-tauri"
DIST="$ROOT/dist"
APP="$DIST/Folio.app"

# Pinned deliberately rather than tracking `latest`. Upstream raised pdfium's
# macOS deployment target to 12.0 in chromium/7469 and to 13.0 later still;
# chromium/7455 is the newest build that still runs on Big Sur (11.0), which is
# the floor Folio targets. Re-check the slices' LC_BUILD_VERSION before moving
# this forward, or the app will silently stop launching on older Macs.
PDFIUM_TAG="chromium/7455"

ARCH="universal"
FEATURES=""
MAKE_DMG=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --arch) ARCH="$2"; shift 2 ;;
        --features) FEATURES="$2"; shift 2 ;;
        --dmg) MAKE_DMG=1; shift ;;
        *) echo "unknown argument: $1" >&2; exit 1 ;;
    esac
done

case "$ARCH" in
    universal) TARGETS=(x86_64-apple-darwin aarch64-apple-darwin) ;;
    x86_64)    TARGETS=(x86_64-apple-darwin) ;;
    arm64)     TARGETS=(aarch64-apple-darwin) ;;
    *) echo "error: --arch must be universal, x86_64 or arm64" >&2; exit 1 ;;
esac

# ── pdfium ───────────────────────────────────────────────────────────────────
# build.rs embeds the shared library into the executable, so it must be present
# before building. The "univ" archive holds both architectures.
DYLIB="$CRATE/pdfium/libpdfium.dylib"
if [[ ! -f "$DYLIB" ]]; then
    echo "==> fetching pdfium ($PDFIUM_TAG)"
    tmp="$(mktemp -d)"
    curl -fsSL -o "$tmp/pdfium.tgz" \
        "https://github.com/bblanchon/pdfium-binaries/releases/download/$PDFIUM_TAG/pdfium-mac-univ.tgz"
    tar -xzf "$tmp/pdfium.tgz" -C "$tmp" lib/libpdfium.dylib
    mkdir -p "$CRATE/pdfium"
    mv "$tmp/lib/libpdfium.dylib" "$DYLIB"
    rm -rf "$tmp"
fi

# ── build ────────────────────────────────────────────────────────────────────
cargo_args=(build --release)
if [[ -n "$FEATURES" ]]; then
    cargo_args+=(--features "$FEATURES")
fi

for target in "${TARGETS[@]}"; do
    echo "==> building $target"
    rustup target add "$target" >/dev/null
    # Big Sur is the floor for both slices: it is the oldest macOS that runs on
    # Apple Silicon, and it matches what the pinned pdfium above supports.
    export MACOSX_DEPLOYMENT_TARGET=11.0
    (cd "$CRATE" && cargo "${cargo_args[@]}" --target "$target")
done

# ── assemble the bundle ──────────────────────────────────────────────────────
echo "==> assembling Folio.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/Frameworks"

# Ship pdfium inside the bundle even though it is also embedded in the binary.
# The embedded copy is extracted to ~/Library/Caches at first run, and macOS
# will not dlopen that copy when Folio itself is quarantined — which is the
# normal state of a freshly downloaded app. The library here is signed as part
# of the bundle, so it always loads; src/pdf.rs looks in Frameworks first.
cp "$DYLIB" "$APP/Contents/Frameworks/libpdfium.dylib"

binaries=()
for target in "${TARGETS[@]}"; do
    binaries+=("$CRATE/target/$target/release/folio")
done
# `lipo -create` with a single input is just a copy, so this covers both modes.
lipo -create "${binaries[@]}" -output "$APP/Contents/MacOS/folio"
chmod +x "$APP/Contents/MacOS/folio"

cp "$CRATE/icons/icon.icns" "$APP/Contents/Resources/folio.icns"

VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$CRATE/Cargo.toml" | head -1)"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>                  <string>Folio</string>
    <key>CFBundleDisplayName</key>           <string>Folio</string>
    <key>CFBundleIdentifier</key>            <string>io.github.hanielulises.folio</string>
    <key>CFBundleExecutable</key>            <string>folio</string>
    <key>CFBundleIconFile</key>              <string>folio</string>
    <key>CFBundlePackageType</key>           <string>APPL</string>
    <key>CFBundleInfoDictionaryVersion</key> <string>6.0</string>
    <key>CFBundleShortVersionString</key>    <string>$VERSION</string>
    <key>CFBundleVersion</key>               <string>$VERSION</string>
    <key>LSMinimumSystemVersion</key>        <string>11.0</string>
    <key>LSApplicationCategoryType</key>     <string>public.app-category.productivity</string>
    <key>NSHighResolutionCapable</key>       <true/>
    <!-- Let macOS keep Folio on the integrated GPU on dual-GPU Intel Macs. -->
    <key>NSSupportsAutomaticGraphicsSwitching</key> <true/>
</dict>
</plist>
PLIST

# ── sign ─────────────────────────────────────────────────────────────────────
# Apple Silicon refuses to execute a binary with no signature at all, and `lipo`
# strips whatever the linker attached, so signing is never optional.
IDENTITY="${FOLIO_SIGN_IDENTITY:--}"
# Sign inside out: nested code first, then the bundle, so the outer signature
# seals the already-signed pdfium. `--deep` is the shortcut for this and Apple
# discourages it for anything that gets notarised.
if [[ "$IDENTITY" == "-" ]]; then
    echo "==> ad-hoc signing (not distributable — see the note at the end)"
    codesign --force --sign - "$APP/Contents/Frameworks/libpdfium.dylib"
    codesign --force --sign - "$APP"
else
    echo "==> signing with: $IDENTITY"
    # The hardened runtime is mandatory for notarisation; the entitlements let
    # the app load the pdfium copy it extracts at runtime.
    codesign --force --timestamp --options runtime \
        --sign "$IDENTITY" "$APP/Contents/Frameworks/libpdfium.dylib"
    codesign --force --timestamp --options runtime \
        --entitlements "$CRATE/macos/Folio.entitlements" \
        --sign "$IDENTITY" "$APP"
fi
# --deep so the nested pdfium is verified too, not just the outer bundle.
codesign --verify --deep --strict "$APP" && echo "signature OK"

# ── notarise ─────────────────────────────────────────────────────────────────
NOTARIZED=0
if [[ -n "${FOLIO_NOTARIZE_APPLE_ID:-}" && "$IDENTITY" != "-" ]]; then
    echo "==> submitting to Apple for notarisation (this takes a few minutes)"
    notarize_zip="$DIST/notarize.zip"
    ditto -c -k --sequesterRsrc --keepParent "$APP" "$notarize_zip"
    xcrun notarytool submit "$notarize_zip" \
        --apple-id "$FOLIO_NOTARIZE_APPLE_ID" \
        --team-id "$FOLIO_NOTARIZE_TEAM_ID" \
        --password "$FOLIO_NOTARIZE_PASSWORD" \
        --wait
    # Stapling attaches the ticket so the app validates even offline.
    xcrun stapler staple "$APP"
    rm -f "$notarize_zip"
    NOTARIZED=1
fi

# ── package ──────────────────────────────────────────────────────────────────
if [[ "$MAKE_DMG" == "1" ]]; then
    echo "==> building disk image"
    DMG="$DIST/folio-macos.dmg"
    stage="$(mktemp -d)"
    # ditto, not cp -R: it preserves the extended attributes and resource forks
    # that a code signature depends on.
    ditto "$APP" "$stage/Folio.app"
    # The Applications symlink is what makes the window a drag-to-install target.
    ln -s /Applications "$stage/Applications"
    if [[ "$NOTARIZED" == "0" ]]; then
        # Without notarisation the first launch needs a manual approval, so say
        # so where the user cannot miss it.
        cat > "$stage/READ ME FIRST.txt" <<'TXT'
Installing Folio
================

1. Drag the Folio icon onto the Applications folder shown beside it.
2. Open your Applications folder and double-click Folio.

The first time, macOS will say Folio "cannot be opened because it is from an
unidentified developer", or that it is damaged. This is because this build is
not signed with a paid Apple Developer certificate — not because anything is
wrong with it. To allow it:

  * Open  System Settings > Privacy & Security
  * Scroll down to the Security section, where you will see a line about Folio
  * Click "Open Anyway", then confirm

You only have to do this once.
TXT
    fi
    rm -f "$DMG"
    hdiutil create -volname "Folio" -srcfolder "$stage" -ov -format UDZO "$DMG" >/dev/null
    rm -rf "$stage"
    # A signed DMG means Gatekeeper checks the image itself, not just the app.
    if [[ "$IDENTITY" != "-" ]]; then
        codesign --force --sign "$IDENTITY" "$DMG"
    fi
    if [[ "$NOTARIZED" == "1" ]]; then
        xcrun stapler staple "$DMG"
    fi
    echo "built $DMG"
fi

echo
echo "Built $APP (arch: $ARCH, version $VERSION)"
if [[ "$NOTARIZED" == "1" ]]; then
    echo "Signed and notarised — it will open on any Mac by double-clicking."
elif [[ "$IDENTITY" != "-" ]]; then
    echo "Signed but NOT notarised — macOS will still warn on a downloaded copy."
else
    echo "NOTE: ad-hoc signed. Fine on this machine, but anyone who downloads it"
    echo "      must approve it in System Settings > Privacy & Security first."
    echo "      Set FOLIO_SIGN_IDENTITY + FOLIO_NOTARIZE_* to avoid that."
fi
