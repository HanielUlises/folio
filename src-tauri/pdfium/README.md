# pdfium shared library

`build.rs` embeds the library found here into the executable, so the right file
for the target platform must be present before building:

| Target  | File               | Archive                 |
|---------|--------------------|-------------------------|
| Linux   | `libpdfium.so`     | `pdfium-linux-x64.tgz`  |
| Windows | `pdfium.dll`       | `pdfium-win-x64.tgz`    |
| macOS   | `libpdfium.dylib`  | `pdfium-mac-univ.tgz`   |

Download pre-built binaries from:
  https://github.com/bblanchon/pdfium-binaries/releases

Only `libpdfium.so` is committed. The Windows and macOS libraries are fetched
during the release build (see `.github/workflows/release.yml` and
`scripts/bundle-macos.sh`); a copy fetched locally is ignored by git.

Example (Linux x64):

    curl -L https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-linux-x64.tgz \
      | tar xz -C /tmp/pdfium
    cp /tmp/pdfium/lib/libpdfium.so ./

Example (macOS — the "univ" archive carries both Intel and Apple Silicon):

    curl -L https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/7455/pdfium-mac-univ.tgz \
      | tar xz -C /tmp/pdfium
    cp /tmp/pdfium/lib/libpdfium.dylib ./

The macOS build is pinned to `chromium/7455` on purpose: it is the last upstream
release whose slices still target macOS 11. `chromium/7469` raised the
deployment target to 12.0 and later builds require 13.0, so tracking `latest`
would quietly drop every Mac that cannot run Ventura. Verify with

    otool -l libpdfium.dylib | grep -A3 LC_BUILD_VERSION

before changing the pin in `scripts/bundle-macos.sh`.

`scripts/bundle-macos.sh` performs the macOS download itself when the file is
missing, so the above is only needed for a plain `cargo build` on a Mac.

To build against a library kept elsewhere, point `FOLIO_PDFIUM_EMBED` at it.
