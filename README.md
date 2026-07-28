# Folio

Folio is a PDF reader and library manager written in Rust. It targets Linux and Windows.

The application is built with egui/eframe for the interface and pdfium-render for PDF handling. Rendering, text extraction, thumbnail generation and search are performed entirely in-process.

## Features

The library view presents a grid of PDF covers. Covers are rendered on a background thread and cached on disk so that subsequent launches remain responsive.

Documents may be organised under one or more topics (colour-coded and shown in the sidebar) and any number of tags. Both topics and tags support filtering, and the library can be shown flat or grouped by topic.

The reader performs lazy per-page rendering and supports zoom, navigation and continuous scrolling. Unused page textures are released to limit memory consumption.

Text can be selected directly on the page and copied. Highlights in five colours are stored as normalised page coordinates and remain correctly aligned at any zoom level. Highlights may be removed by clicking them.

The complete library state (topics, tags, document entries and highlights) can be exported or imported as a single JSON file. Persistent data is stored in the platform application data directory as `folio-data.json`.

## Screenshots

The library view groups PDF covers by topic, with topics and tags in the sidebar:

![Folio library view](docs/screenshots/library.png)

The reader offers a table-of-contents panel, page navigation, zoom and five highlight colours:

![Folio reader view](docs/screenshots/reader.png)

## Download

Pre-built binaries are available from the [Releases](https://github.com/HanielUlises/folio/releases) page.

- **Linux** (`folio-linux-x86_64.tar.gz`): extract and run the `folio` executable. The required `libpdfium.so` is included alongside it.
- **Windows** (`folio-windows-x86_64.zip`): extract and run `folio.exe`. Keep the bundled `pdfium.dll` in the same folder as the executable.

## Requirements

- Rust 1.97.1 (specified in `rust-toolchain.toml`)
- A working OpenGL implementation (on Linux: X11 or Wayland)
- The pdfium shared library next to the executable — `libpdfium.so` on Linux, `pdfium.dll` on Windows. Both are included in the release packages; a Linux copy is vendored under `src-tauri/pdfium/` for building from source.

At runtime the application searches for the pdfium library in the following order: `$FOLIO_PDFIUM_DIR`, the directory containing the executable (including a `pdfium/` subdirectory), the vendored copy under `src-tauri/pdfium/`, and finally the system library path.