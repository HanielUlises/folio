# Folio

[![Latest release](https://img.shields.io/github/v/release/HanielUlises/folio?sort=semver)](https://github.com/HanielUlises/folio/releases)
[![License: MIT](https://img.shields.io/github/license/HanielUlises/folio)](LICENSE)
![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-informational)
![Built with Rust](https://img.shields.io/badge/built%20with-Rust-orange?logo=rust&logoColor=white)

Folio is a PDF reader and library manager written in Rust. It targets Linux, macOS and Windows.

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

Folio ships three themes: dark, light and sepia:

| Dark | Light | Sepia |
|:----:|:-----:|:-----:|
| ![Dark theme](docs/screenshots/theme-dark.png) | ![Light theme](docs/screenshots/theme-light.png) | ![Sepia theme](docs/screenshots/theme-sepia.png) |

## Download

Pre-built binaries are available from the [Releases](https://github.com/HanielUlises/folio/releases) page.

- **Linux** (`folio-linux-x86_64.tar.gz`): extract and run the `folio` executable.
- **macOS** (`folio-macos-universal.dmg`): open it and drag Folio onto the
  Applications folder. A universal build — one file for both Apple Silicon and
  Intel Macs — requiring macOS 11 Big Sur or later.
- **Windows** (`folio.exe`): download and run it directly no installation, no separate files.

The pdfium library is embedded in the executable and extracted to a per-user cache directory on first run, so nothing needs to be kept alongside it.

#### First launch on macOS

Unless the release was built with an Apple Developer ID (see below), macOS
refuses to open it straight away and reports that Folio is from an
unidentified developer, or that it is damaged. Nothing is wrong with the
download — Apple shows this for any app that has not been notarised. To allow
it, once:

1. Open **System Settings → Privacy & Security**.
2. Scroll to **Security**, where a line about Folio appears.
3. Click **Open Anyway** and confirm.

The same instructions are included inside the disk image. On macOS 14 and
earlier you can instead right-click the app and choose **Open**; macOS 15
Sequoia removed that shortcut.

To remove the step entirely, the release must be signed with a Developer ID
certificate and notarised by Apple, which needs a paid Apple Developer account.
The build already supports this: set the `APPLE_CERTIFICATE_P12`,
`APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`,
`APPLE_TEAM_ID` and `APPLE_APP_PASSWORD` repository secrets and the release
workflow signs, notarises and staples automatically — after which the DMG opens
with a plain double-click and no warning.

### Application menu integration

Folio registers itself the first time it runs, so it shows up in your
application menu / search by name with no separate installer:

- **Linux:** a per-user `.desktop` entry and icon are written under
  `~/.local/share` (works on any freedesktop desktop GNOME, KDE, …).
- **Windows:** a Start Menu shortcut is created so Folio appears in search.
- **macOS:** nothing to register — `Folio.app` is indexed by Spotlight and
  Launchpad as soon as it is in Applications.

For a manual or scripted install from source on Linux, `scripts/install-desktop.sh`
does the same thing explicitly (resolving paths relative to the repository).

## Building from source

Building needs a Rust toolchain (the pinned version is installed automatically
from `rust-toolchain.toml`) and the pdfium shared library for the target
platform under `src-tauri/pdfium/` — see the [README there](src-tauri/pdfium/README.md).

```sh
cd src-tauri && cargo build --release        # Linux, Windows
```

On macOS, use the bundling script instead — it fetches pdfium, builds both
architectures, merges them and produces `dist/Folio.app`:

```sh
scripts/bundle-macos.sh                      # universal
scripts/bundle-macos.sh --arch arm64         # this machine only, faster
scripts/bundle-macos.sh --dmg                # also build dist/folio-macos.dmg
```

pdfium is pinned there to a build that still supports macOS 11; newer upstream
releases raise the requirement to macOS 12 and then 13, which would silently
drop older Macs. Check the deployment target of both slices before bumping it.

Xcode command line tools are required (`xcode-select --install`).

## Google Drive (experimental)

Folio can connect to Google Drive to browse and open your PDFs directly. This
integration is **experimental and currently invitation-only**
