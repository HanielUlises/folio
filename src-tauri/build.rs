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

    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("pdfium_embedded.bin");
    fs::copy(&src, &out).unwrap_or_else(|e| {
        panic!(
            "failed to read pdfium library for embedding from {} ({e}).\n\
             Place `{libname}` under src-tauri/pdfium/ or point FOLIO_PDFIUM_EMBED at it.",
            src.display()
        )
    });
}
