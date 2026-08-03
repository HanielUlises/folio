#!/usr/bin/env python3
"""Build src-tauri/icons/icon.icns from the master PNG logo.

macOS reads the app icon from a multi-resolution `.icns` bundle; Finder, the
Dock, Spotlight and the ⌘-Tab switcher each pick a different slice. Run this
whenever the logo changes so all of them stay in sync:

    python3 scripts/make-icns.py

Requires Pillow. Writes the same PNG-based chunk types `iconutil` emits, so the
result is byte-compatible with a bundle built on macOS itself.
"""

import io
import struct
import sys
from pathlib import Path

try:
    from PIL import Image
except ImportError:
    sys.exit("Pillow is required: pip install pillow")

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "src-tauri" / "icons" / "128x128@2x.png"
DST = ROOT / "src-tauri" / "icons" / "icon.icns"

# (icns chunk type, pixel size). The @2x entries repeat a size deliberately:
# macOS treats e.g. ic13 as "128pt at 2x", which is a different slot from ic08
# ("256pt at 1x") even though both hold a 256px image.
SLICES = [
    ("icp4", 16),   # 16pt @1x
    ("icp5", 32),   # 32pt @1x
    ("ic11", 32),   # 16pt @2x
    ("icp6", 64),   # 64pt @1x
    ("ic12", 64),   # 32pt @2x
    ("ic07", 128),  # 128pt @1x
    ("ic08", 256),  # 256pt @1x
    ("ic13", 256),  # 128pt @2x
]


def main() -> None:
    master = Image.open(SRC).convert("RGBA")
    if master.width != master.height:
        sys.exit(f"{SRC} must be square, got {master.width}x{master.height}")

    chunks = bytearray()
    for kind, size in SLICES:
        if size > master.width:
            sys.exit(f"{SRC} is only {master.width}px; cannot produce a {size}px slice")
        img = master if size == master.width else master.resize((size, size), Image.LANCZOS)
        buf = io.BytesIO()
        img.save(buf, format="PNG", optimize=True)
        png = buf.getvalue()
        # Each chunk is: 4-byte type, big-endian u32 length *including* this
        # 8-byte header, then the payload.
        chunks += kind.encode("ascii") + struct.pack(">I", len(png) + 8) + png

    icns = b"icns" + struct.pack(">I", len(chunks) + 8) + bytes(chunks)
    DST.write_bytes(icns)
    print(f"wrote {DST.relative_to(ROOT)} ({len(icns)} bytes, {len(SLICES)} slices)")


if __name__ == "__main__":
    main()
