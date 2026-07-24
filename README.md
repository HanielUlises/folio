# Folio

Folio is a PDF reader and library manager written in Rust. It targets Linux and other desktop platforms.

The application is built with egui/eframe for the interface and pdfium-render for PDF handling. Rendering, text extraction, thumbnail generation and search are performed entirely in-process.

## Features

The library view presents a grid of PDF covers. Covers are rendered on a background thread and cached on disk so that subsequent launches remain responsive.

Documents may be organised with a single topic (colour-coded and shown in the sidebar) and any number of tags. Both topics and tags support filtering.

The reader performs lazy per-page rendering and supports zoom, navigation and continuous scrolling. Unused page textures are released to limit memory consumption.

Text can be selected directly on the page and copied. Highlights in five colours are stored as normalised page coordinates and remain correctly aligned at any zoom level. Highlights may be removed by clicking them.

The complete library state (topics, tags, document entries and highlights) can be exported or imported as a single JSON file. Persistent data is stored in the platform application data directory as `folio-data.json`.

## Download

Pre-built Linux binaries are available from the [Releases](https://github.com/<user>/<repo>/releases) page. Extract the archive and run the `folio` executable. The required `libpdfium.so` is included.

## Requirements

- Rust 1.97.1 (specified in `rust-toolchain.toml`)
- A working OpenGL implementation (X11 or Wayland)
- `libpdfium.so` (provided in the release package or under `src-tauri/pdfium/` when building from source)

At runtime the application searches for the pdfium library in the following order: `$FOLIO_PDFIUM_DIR`, the directory containing the executable (including a `pdfium/` subdirectory), the vendored copy under `src-tauri/pdfium/`, and finally the system library path.