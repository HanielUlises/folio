//! Embed the pdfium shared library into the binary.
//!
//! pdfium is loaded dynamically at runtime, so the executable needs the shared
//! library reachable on disk. To ship a single self-contained executable we
//! copy the platform library into `OUT_DIR` here; `src/pdf.rs` then embeds it
//! with `include_bytes!` and extracts it to a cache directory on first run.
//!
//! The library is taken from `$FOLIO_PDFIUM_EMBED` when set, otherwise from the
//! vendored `pdfium/<platform library name>` next to this build script.

use std::{env, fs, path::PathBuf};

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let libname = match target_os.as_str() {
        "windows" => "pdfium.dll",
        "macos" => "libpdfium.dylib",
        _ => "libpdfium.so",
    };

    let src = match env::var_os("FOLIO_PDFIUM_EMBED") {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("pdfium")
            .join(libname),
    };

    println!("cargo:rerun-if-env-changed=FOLIO_PDFIUM_EMBED");
    println!("cargo:rerun-if-changed={}", src.display());

    // Upstream archive that provides the library for this target, so the error
    // below tells the reader exactly what to download.
    let archive = match (target_os.as_str(), env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default().as_str()) {
        ("windows", _) => "pdfium-win-x64.tgz",
        // The macOS "univ" archive carries both x86_64 and arm64 slices, so one
        // download covers Intel and Apple Silicon builds alike.
        ("macos", _) => "pdfium-mac-univ.tgz",
        (_, "aarch64") => "pdfium-linux-arm64.tgz",
        _ => "pdfium-linux-x64.tgz",
    };

    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("pdfium_embedded.bin");
    fs::copy(&src, &out).unwrap_or_else(|e| {
        panic!(
            "failed to read pdfium library for embedding from {} ({e}).\n\
             Place `{libname}` under src-tauri/pdfium/ or point FOLIO_PDFIUM_EMBED at it.\n\
             Prebuilt libraries: https://github.com/bblanchon/pdfium-binaries/releases \
             (this target needs {archive}).",
            src.display()
        )
    });

    // Embed the application icon into folio.exe so Windows shows the Folio logo
    // in Explorer, the taskbar and the title bar. No effect on other platforms.
    if target_os == "windows" {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icons/icon.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=failed to embed Windows icon: {e}");
        }
    }
}
