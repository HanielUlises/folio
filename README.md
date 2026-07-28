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

### Themes

Folio ships three themes — dark, light and sepia:

| Dark | Light | Sepia |
|:----:|:-----:|:-----:|
| ![Dark theme](docs/screenshots/theme-dark.png) | ![Light theme](docs/screenshots/theme-light.png) | ![Sepia theme](docs/screenshots/theme-sepia.png) |

## Download

Pre-built binaries are available from the [Releases](https://github.com/HanielUlises/folio/releases) page.

- **Linux** (`folio-linux-x86_64.tar.gz`): extract and run the `folio` executable.
- **Windows** (`folio.exe`): download and run it directly — no installation, no separate files.

The pdfium library is embedded in the executable and extracted to a per-user cache directory on first run, so nothing needs to be kept alongside it.
