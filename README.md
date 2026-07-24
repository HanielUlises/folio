# Folio

Offline PDF reader and library manager for Linux (and other desktops).

Native Rust application built with egui/eframe (GPU-accelerated, no webview) and pdfium-render. All rendering, text extraction, thumbnails and search happen in-process. No web frontend, no pdf.js, no CDN, and no network access at runtime.

## Features

- Library grid with rendered PDF covers. Covers are generated on a background thread and cached to disk.
- Topics — one colour-coded category per PDF, shown in the sidebar. Sort and filter by topic.
- Tags — multiple colour-coded labels per PDF with one-click filtering.
- Reader with lazy per-page rendering, zoom, page navigation and smooth scrolling. Off-screen page textures are freed to keep memory usage bounded.
- Text selection — drag to select text on the page; copy with the toolbar button or Ctrl+C.
- Highlighting in five colours. Highlights are stored as normalised page coordinates so they stay aligned at any zoom. Click a highlight to remove it.
- Portable data — export / import the full library (topics, tags, entries, highlights) as a single JSON file. Data lives in the OS app-data directory as `folio-data.json`.

## Requirements

- Rust (pinned to 1.97.1 via `rust-toolchain.toml`)
- Working OpenGL stack (uses the glow backend; X11 and Wayland supported)
- `libpdfium.so` (vendored under `src-tauri/pdfium/`)

At runtime Folio searches for the pdfium library in this order:
1. `$FOLIO_PDFIUM_DIR`
2. Directory of the executable (and its `pdfium/` subdirectory)
3. Vendored `src-tauri/pdfium/`
4. System library path