# Folio

A clean, offline PDF reader and organiser for Linux (and other desktops). Folio
is a **pure-Rust native application** built on [`egui`/`eframe`](https://github.com/emilk/egui)
(GPU-accelerated, no webview) with a Rust PDF engine (`pdfium-render`). All
rendering, text extraction, thumbnails and search happen in-process in Rust —
there is no web frontend, no `pdf.js`, no CDN, and no network access at runtime.

## Features

- **Library** grid with rendered **PDF covers**. Covers are generated on a
  background thread and cached to disk, so the library never blocks and opens
  instantly on later launches.
- **Topics** — one colour-coded category per PDF, shown in the sidebar; sort and
  filter your library by topic.
- **Tags** — many colour-coded, cross-cutting labels per PDF, with one-click
  filtering.
- **Reader** with lazy per-page rendering, zoom, page navigation and smooth
  scrolling on large books (off-screen page textures are freed to bound memory).
- **Text selection** — drag to select text directly on the page; copy with the
  toolbar button or `Ctrl+C`.
- **Highlighting** in five colours. Highlights are stored as normalised page
  coordinates, so they stay aligned at any zoom; click a highlight to remove it.
- **Portable data** — Export / Import your whole library (topics, tags, entries,
  highlights) as a single JSON file. Data lives in the OS app-data dir as
  `folio-data.json`.

## Requirements

- Rust (pinned to 1.97.1 via `rust-toolchain.toml`).
- A working OpenGL stack (Folio uses the `glow` backend; X11 and Wayland are both
  supported).
- `src-tauri/pdfium/libpdfium.so` — the pdfium shared library (already vendored
  in this repo). To refresh it, download the Linux x64 build from
  [bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries/releases)
  and drop `libpdfium.so` into `src-tauri/pdfium/`. At runtime Folio searches, in
  order: `$FOLIO_PDFIUM_DIR`, the executable's directory (and its `pdfium/`
  subdir), the vendored `src-tauri/pdfium/`, and finally the system library.

## Run

```bash
cd src-tauri
cargo run --release
```

## Build

```bash
cd src-tauri
cargo build --release      # binary at src-tauri/target/release/folio
```

When distributing, ship `libpdfium.so` next to the `folio` binary (or in a
`pdfium/` subdirectory beside it).

## Verify the PDF engine

```bash
cd src-tauri
cargo run --example pdfium_check -- /path/to/some.pdf
```

Confirms pdfium loads, renders a page, and extracts text without launching the GUI.

There is also a headless test that drives the background engine end-to-end
(open → render → glyph boxes → cover cache). Point it at any PDF:

```bash
cd src-tauri
FOLIO_TEST_PDF=/path/to/some.pdf cargo test -- --test-threads=1
```
