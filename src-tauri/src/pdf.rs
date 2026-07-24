//! pdfium engine wrapper.
//!
//! We bind pdfium once and *leak* the handle to obtain a `&'static Pdfium`.
//! This lets us keep a `PdfDocument<'static>` open for the lifetime of an open
//! book — pages, text and bounds are then read straight from the parsed
//! document with no re-parsing per page, which keeps scrolling fast.

use pdfium_render::prelude::*;
use std::path::PathBuf;

/// One rendered page as RGBA8, ready to upload as an egui texture.
pub struct PageImage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// A single glyph's box in page points, converted to a top-left origin.
#[derive(Clone, Copy)]
pub struct CharBox {
    pub ch: char,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub struct Engine {
    pdfium: &'static Pdfium,
}

pub struct Doc {
    inner: PdfDocument<'static>,
    pub page_count: u32,
}

impl Engine {
    pub fn new() -> Result<Self, String> {
        // Search a few likely locations for the shared library.
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(d) = std::env::var_os("FOLIO_PDFIUM_DIR") {
            dirs.push(PathBuf::from(d));
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(p) = exe.parent() {
                dirs.push(p.to_path_buf());
                dirs.push(p.join("pdfium"));
            }
        }
        dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("pdfium"));

        let mut bindings = None;
        for d in &dirs {
            if let Ok(b) =
                Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(d))
            {
                bindings = Some(b);
                break;
            }
        }
        let bindings = bindings
            .or_else(|| Pdfium::bind_to_system_library().ok())
            .ok_or_else(|| "libpdfium.so not found".to_string())?;

        let pdfium: &'static Pdfium = Box::leak(Box::new(Pdfium::new(bindings)));
        Ok(Self { pdfium })
    }

    pub fn open(&self, path: &str) -> Result<Doc, String> {
        let inner = self
            .pdfium
            .load_pdf_from_file(path, None)
            .map_err(|e| format!("{e:?}"))?;
        let page_count = inner.pages().len() as u32;
        Ok(Doc { inner, page_count })
    }

    /// Render page 0 of `path` at `target_w` pixels wide, for a library cover.
    pub fn thumbnail(&self, path: &str, target_w: u32) -> Result<PageImage, String> {
        let doc = self.open(path)?;
        let (w_pts, _) = doc.page_size(0).ok_or_else(|| "empty document".to_string())?;
        let scale = (target_w as f32 / w_pts).max(0.01);
        doc.render(0, scale)
    }
}

impl Doc {
    /// Page size in points (unscaled).
    pub fn page_size(&self, page_index: u32) -> Option<(f32, f32)> {
        let pages = self.inner.pages();
        let page = pages.get(page_index as u16).ok()?;
        Some((page.width().value, page.height().value))
    }

    /// Render a page at `scale` pixels-per-point, composited over white.
    pub fn render(&self, page_index: u32, scale: f32) -> Result<PageImage, String> {
        let pages = self.inner.pages();
        let page = pages.get(page_index as u16).map_err(|e| format!("{e:?}"))?;

        let w_pts = page.width().value;
        let h_pts = page.height().value;
        let w_px = ((w_pts * scale).round() as i32).max(1);
        let h_px = ((h_pts * scale).round() as i32).max(1);

        let bitmap = page
            .render_with_config(
                &PdfRenderConfig::new()
                    .set_target_width(w_px)
                    .set_target_height(h_px),
            )
            .map_err(|e| format!("{e:?}"))?;

        let width = bitmap.width() as usize;
        let height = bitmap.height() as usize;
        let mut rgba = bitmap.as_rgba_bytes();

        // Composite over white so transparent PDF backgrounds read as paper.
        for px in rgba.chunks_exact_mut(4) {
            let a = px[3] as u32;
            if a != 255 {
                let ia = 255 - a;
                px[0] = ((px[0] as u32 * a + 255 * ia) / 255) as u8;
                px[1] = ((px[1] as u32 * a + 255 * ia) / 255) as u8;
                px[2] = ((px[2] as u32 * a + 255 * ia) / 255) as u8;
                px[3] = 255;
            }
        }

        Ok(PageImage { width, height, rgba })
    }

    /// Per-character boxes for a page, in page points with a top-left origin.
    pub fn char_boxes(&self, page_index: u32) -> Vec<CharBox> {
        let pages = self.inner.pages();
        let Ok(page) = pages.get(page_index as u16) else {
            return Vec::new();
        };
        let page_h = page.height().value;
        let Ok(text) = page.text() else {
            return Vec::new();
        };
        let chars = text.chars();
        let mut out = Vec::with_capacity(chars.len());
        for i in 0..chars.len() {
            let Ok(c) = chars.get(i) else { continue };
            let ch = c.unicode_char().unwrap_or(' ');
            let Ok(b) = c.loose_bounds() else { continue };
            let left = b.left().value;
            let right = b.right().value;
            let top = b.top().value;
            let bottom = b.bottom().value;
            out.push(CharBox {
                ch,
                x: left,
                y: page_h - top, // flip to top-left origin
                w: (right - left).max(0.0),
                h: (top - bottom).max(0.0),
            });
        }
        out
    }
}
