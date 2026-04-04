# pdfium shared library

Place `libpdfium.so` (Linux), `pdfium.dll` (Windows), or `libpdfium.dylib` (macOS) here.

Download pre-built binaries from:
  https://github.com/bblanchon/pdfium-binaries/releases

Example (Linux x64):
  curl -L https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-linux-x64.tgz \
    | tar xz -C /tmp/pdfium
  cp /tmp/pdfium/lib/libpdfium.so ./

The Tauri bundler will copy this file next to the app binary via tauri.conf.json bundle.resources.
