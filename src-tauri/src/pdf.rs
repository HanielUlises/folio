//! pdfium engine wrapper.
//!
//! We bind pdfium once and *leak* the handle to obtain a `&'static Pdfium`.
//! This lets us keep a `PdfDocument<'static>` open for the lifetime of an open
//! book — pages, text and bounds are then read straight from the parsed
//! document with no re-parsing per page, which keeps scrolling fast.

use pdfium_render::prelude::*;
use std::path::PathBuf;

/// The pdfium shared library, embedded so the executable is self-contained.
/// `build.rs` places the platform library here.
const EMBEDDED_PDFIUM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/pdfium_embedded.bin"));

/// Extract the embedded pdfium library to a cache directory and return the
/// directory it was written to, so it can be added to the load search path.
///
/// Best-effort: returns `None` if the library could not be written (e.g. a
/// read-only cache), in which case the caller falls back to the other search
/// locations. The extracted copy is reused across runs when it already matches
/// the embedded bytes.
fn extracted_pdfium_dir() -> Option<PathBuf> {
    let libname = Pdfium::pdfium_platform_library_name();
    let dir = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("folio")
        .join("pdfium");
    let target = dir.join(&libname);

    let up_to_date = std::fs::metadata(&target)
        .map(|m| m.len() == EMBEDDED_PDFIUM.len() as u64)
        .unwrap_or(false);
    if !up_to_date {
        std::fs::create_dir_all(&dir).ok()?;
        // Write to a temp file and rename so a concurrent run never sees a
        // half-written library.
        let mut tmp_name = libname.clone();
        tmp_name.push(".tmp");
        let tmp = dir.join(&tmp_name);
        std::fs::write(&tmp, EMBEDDED_PDFIUM).ok()?;
        std::fs::rename(&tmp, &target).ok()?;
    }
    Some(dir)
}

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

/// One entry in a document's table of contents (outline / bookmarks).
#[derive(Clone, Debug)]
pub struct OutlineItem {
    pub level: u32,
    pub title: String,
    pub page: Option<u32>, // 0-based target page, if the bookmark has a destination
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
        // Search a few likely locations for the shared library. The embedded
        // copy, extracted to the cache directory, is tried first so a bare
        // executable works with no library shipped alongside it.
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(d) = std::env::var_os("FOLIO_PDFIUM_DIR") {
            dirs.push(PathBuf::from(d));
        }
        if let Some(d) = extracted_pdfium_dir() {
            dirs.push(d);
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
    /// Page size in points (unscaled), as displayed — crop box, rotated.
    pub fn page_size(&self, page_index: u32) -> Option<(f32, f32)> {
        let pages = self.inner.pages();
        let page = pages.get(page_index as u16).ok()?;
        Some(self.display_size(&page))
    }

    /// Displayed page size in points: the crop box, with width/height swapped
    /// for 90°/270° rotation — the same frame `char_boxes` maps glyphs into.
    fn display_size(&self, page: &PdfPage) -> (f32, f32) {
        let (l, b, r, t) = self.effective_box(page);
        let (bw, bh) = ((r - l).max(1.0), (t - b).max(1.0));
        match page_rotation_degrees(page) {
            90 | 270 => (bh, bw),
            _ => (bw, bh),
        }
    }

    /// Render a page at `scale` pixels-per-point, composited over white.
    pub fn render(&self, page_index: u32, scale: f32) -> Result<PageImage, String> {
        let pages = self.inner.pages();
        let page = pages.get(page_index as u16).map_err(|e| format!("{e:?}"))?;

        let (w_pts, h_pts) = self.display_size(&page);
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

    /// Per-character boxes for a page, in *rendered-image* points with a
    /// top-left origin — i.e. aligned with what `render` produces, so overlays
    /// sit exactly on the glyphs. This accounts for the crop box (glyph coords
    /// are in page user space, which may have a non-zero origin) and for page
    /// rotation (pdfium renders rotated; the text layout is not).
    pub fn char_boxes(&self, page_index: u32) -> Vec<CharBox> {
        let pages = self.inner.pages();
        let Ok(page) = pages.get(page_index as u16) else {
            return Vec::new();
        };

        // Effective visible box (crop box if present, else media box).
        let (box_left, box_bottom, box_right, box_top) = self.effective_box(&page);
        let bw = (box_right - box_left).max(1.0);
        let bh = (box_top - box_bottom).max(1.0);
        let rot = page_rotation_degrees(&page);

        let Ok(text) = page.text() else {
            return Vec::new();
        };
        let chars = text.chars();
        let mut out = Vec::with_capacity(chars.len());
        for i in 0..chars.len() {
            let Ok(c) = chars.get(i) else { continue };
            let ch = c.unicode_char().unwrap_or(' ');
            let Ok(b) = c.loose_bounds() else { continue };
            // Corners relative to the crop-box origin, in y-up user space.
            let ul = b.left().value - box_left;
            let ur = b.right().value - box_left;
            let vb = b.bottom().value - box_bottom;
            let vt = b.top().value - box_bottom;
            // Map every corner into rendered (top-left, y-down) space, then take
            // the axis-aligned bounding box — correct for any 90° rotation.
            let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for (u, v) in [(ul, vb), (ur, vb), (ul, vt), (ur, vt)] {
                let (x, y) = map_rot(u, v, bw, bh, rot);
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
            out.push(CharBox {
                ch,
                x: x0,
                y: y0,
                w: (x1 - x0).max(0.0),
                h: (y1 - y0).max(0.0),
            });
        }
        out
    }

    /// The document's table of contents, flattened depth-first with levels.
    pub fn outline(&self) -> Vec<OutlineItem> {
        let mut out = Vec::new();
        let bookmarks = self.inner.bookmarks();
        let mut node = bookmarks.root();
        while let Some(bm) = node {
            let next = bm.next_sibling();
            collect_bookmarks(&bm, 0, &mut out);
            node = next;
            if out.len() > 5000 {
                break; // guard against pathological / cyclic outlines
            }
        }
        out
    }

    /// The visible page box (crop box, falling back to media box) as
    /// `(left, bottom, right, top)` in user-space points.
    fn effective_box(&self, page: &PdfPage) -> (f32, f32, f32, f32) {
        let b = page
            .boundaries()
            .crop()
            .or_else(|_| page.boundaries().media())
            .map(|bb| bb.bounds);
        match b {
            Ok(r) => (r.left().value, r.bottom().value, r.right().value, r.top().value),
            Err(_) => (0.0, 0.0, page.width().value, page.height().value),
        }
    }
}

/// Recursively flatten a bookmark subtree into `out`, indenting by `level`.
fn collect_bookmarks(bm: &PdfBookmark, level: u32, out: &mut Vec<OutlineItem>) {
    if level > 32 || out.len() > 5000 {
        return;
    }
    let title = bm.title().unwrap_or_default();
    let page = bm
        .destination()
        .and_then(|d| d.page_index().ok())
        .map(|p| p as u32);
    if !title.trim().is_empty() {
        out.push(OutlineItem { level, title, page });
    }
    let mut child = bm.first_child();
    while let Some(c) = child {
        let next = c.next_sibling();
        collect_bookmarks(&c, level + 1, out);
        child = next;
    }
}

/// Page rotation as a multiple of 90°, matching how pdfium renders the bitmap.
fn page_rotation_degrees(page: &PdfPage) -> u16 {
    match page.rotation() {
        Ok(PdfPageRenderRotation::Degrees90) => 90,
        Ok(PdfPageRenderRotation::Degrees180) => 180,
        Ok(PdfPageRenderRotation::Degrees270) => 270,
        _ => 0,
    }
}

/// Map a crop-relative, y-up point `(u, v)` in a `bw × bh` box into rendered
/// image space (top-left origin, y-down), applying the page's display rotation.
fn map_rot(u: f32, v: f32, bw: f32, bh: f32, rot: u16) -> (f32, f32) {
    match rot {
        90 => (v, u),
        180 => (bw - u, v),
        270 => (bh - v, bw - u),
        _ => (u, bh - v),
    }
}
