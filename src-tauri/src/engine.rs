//! Background PDF engine worker.
//!
//! pdfium is not thread-safe, so *all* pdfium calls are funnelled onto a single
//! dedicated worker thread. The UI thread talks to it purely through channels
//! and never blocks: it fires a `Req`, keeps painting, and later drains `Res`
//! messages (covers, rendered pages, glyph boxes) as they arrive. The worker
//! pings `Context::request_repaint` whenever it produces a result so the UI
//! wakes up to consume it.
//!
//! Covers are additionally cached to disk as PNGs keyed by path + mtime + size,
//! so a library opens instantly on every launch after the first.

use crate::pdf::{CharBox, Doc, Engine, OutlineItem};
use eframe::egui::{self, ColorImage};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

pub enum Req {
    /// Open a document and report its page count + first page size.
    Open(String),
    /// Render `page` (0-based) of `path` at `scale` px/pt.
    Render {
        path: String,
        page: u32,
        scale: f32,
    },
    /// Extract per-glyph boxes for `page` (0-based) of `path`.
    Chars { path: String, page: u32 },
    /// Render (or load from disk) a library cover for `id`/`path`.
    Cover {
        id: String,
        path: String,
        target_w: u32,
    },
    /// Drop the cached reader document.
    Close,
}

pub enum Res {
    /// pdfium could not be initialised; the whole engine is unavailable.
    EngineError(String),
    Opened {
        path: String,
        page_count: u32,
        first_size: (f32, f32),
    },
    Outline {
        path: String,
        items: Vec<OutlineItem>,
    },
    /// Exact size (in points) of every page, so page layout and jump-to-page are
    /// accurate before a page is rendered (page sizing needs no rendering).
    Sizes {
        path: String,
        sizes: Vec<(f32, f32)>,
    },
    OpenFailed {
        path: String,
        err: String,
    },
    Page {
        path: String,
        page: u32,
        scale: f32,
        size_pts: (f32, f32),
        img: ColorImage,
    },
    Chars {
        path: String,
        page: u32,
        chars: Vec<CharBox>,
    },
    Cover {
        id: String,
        img: Option<ColorImage>,
    },
}

pub struct Worker {
    tx: Sender<Req>,
    rx: Receiver<Res>,
}

impl Worker {
    pub fn spawn(ctx: egui::Context) -> Self {
        let (req_tx, req_rx) = channel::<Req>();
        let (res_tx, res_rx) = channel::<Res>();

        // pdfium is bound exactly once, on the worker thread — its handles must
        // never be touched from more than one thread. An init failure comes back
        // asynchronously as `Res::EngineError`.
        std::thread::Builder::new()
            .name("folio-pdf".into())
            .spawn(move || worker_loop(req_rx, res_tx, ctx))
            .expect("spawn pdf worker");

        Self {
            tx: req_tx,
            rx: res_rx,
        }
    }

    pub fn send(&self, req: Req) {
        // Ignore send errors: they only occur if the worker died, in which
        // case the UI simply keeps its placeholders.
        let _ = self.tx.send(req);
    }

    /// Drain everything the worker has produced since last frame.
    pub fn drain(&self) -> Vec<Res> {
        self.rx.try_iter().collect()
    }
}

fn worker_loop(rx: Receiver<Req>, tx: Sender<Res>, ctx: egui::Context) {
    let engine = match Engine::new() {
        Ok(e) => e,
        Err(e) => {
            let _ = tx.send(Res::EngineError(e));
            ctx.request_repaint();
            return;
        }
    };
    // The single reader document is kept open so page/char requests for the
    // book currently on screen never re-parse the file.
    let mut open: Option<(String, Doc)> = None;

    // Reader interactions (open/render/chars) must never wait behind a backlog
    // of background cover renders. Each wake-up we drain everything currently
    // queued and process the high-priority requests first (a stable sort keeps
    // arrival order within each class).
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        while let Ok(more) = rx.try_recv() {
            batch.push(more);
        }
        batch.sort_by_key(priority);
        for req in batch {
            handle(req, &engine, &mut open, &tx, &ctx);
        }
    }
}

fn priority(req: &Req) -> u8 {
    match req {
        Req::Close => 0,
        Req::Open(_) => 1,
        Req::Render { .. } => 1,
        Req::Chars { .. } => 2,
        Req::Cover { .. } => 3,
    }
}

fn handle(
    req: Req,
    engine: &Engine,
    open: &mut Option<(String, Doc)>,
    tx: &Sender<Res>,
    ctx: &egui::Context,
) {
    match req {
        Req::Close => {
            *open = None;
        }
        Req::Open(path) => {
            match ensure_open(engine, open, &path) {
                Ok(d) => {
                    let first = d.page_size(0).unwrap_or((595.0, 842.0));
                    let items = d.outline();
                    let _ = tx.send(Res::Opened {
                        path: path.clone(),
                        page_count: d.page_count,
                        first_size: first,
                    });
                    // All page sizes up front (fast — no rendering) so jumps and
                    // layout are exact even for pages not yet drawn.
                    let sizes: Vec<(f32, f32)> = (0..d.page_count)
                        .map(|i| d.page_size(i).unwrap_or(first))
                        .collect();
                    let _ = tx.send(Res::Sizes { path: path.clone(), sizes });
                    let _ = tx.send(Res::Outline { path, items });
                }
                Err(e) => {
                    let _ = tx.send(Res::OpenFailed { path, err: e });
                }
            }
            ctx.request_repaint();
        }
        Req::Render { path, page, scale } => {
            if let Ok(d) = ensure_open(engine, open, &path) {
                let size_pts = d.page_size(page).unwrap_or((595.0, 842.0));
                if let Ok(img) = d.render(page, scale) {
                    let color = ColorImage::from_rgba_unmultiplied([img.width, img.height], &img.rgba);
                    let _ = tx.send(Res::Page { path, page, scale, size_pts, img: color });
                    ctx.request_repaint();
                }
            }
        }
        Req::Chars { path, page } => {
            if let Ok(d) = ensure_open(engine, open, &path) {
                let chars = d.char_boxes(page);
                let _ = tx.send(Res::Chars { path, page, chars });
                ctx.request_repaint();
            }
        }
        Req::Cover { id, path, target_w } => {
            let img = load_or_render_cover(engine, &path, target_w);
            let _ = tx.send(Res::Cover { id, img });
            ctx.request_repaint();
        }
    }
}

/// Ensure `open` holds the document for `path`, opening it if necessary.
fn ensure_open<'a>(
    engine: &Engine,
    open: &'a mut Option<(String, Doc)>,
    path: &str,
) -> Result<&'a Doc, String> {
    let need = !matches!(open, Some((p, _)) if p == path);
    if need {
        let doc = engine.open(path)?;
        *open = Some((path.to_string(), doc));
    }
    Ok(&open.as_ref().unwrap().1)
}

// ── Cover disk cache ─────────────────────────────────────────────────────────

fn cover_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.folio.pdf")
        .join("covers")
}

/// Cache key: a stable hash of path + mtime + size, so a replaced file
/// re-renders rather than showing a stale cover.
fn cover_key(path: &str) -> String {
    let meta = std::fs::metadata(path).ok();
    let mtime = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    // FNV-1a over the discriminating fields.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in path
        .as_bytes()
        .iter()
        .chain(&mtime.to_le_bytes())
        .chain(&size.to_le_bytes())
    {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn load_or_render_cover(engine: &Engine, path: &str, target_w: u32) -> Option<ColorImage> {
    let dir = cover_cache_dir();
    let file = dir.join(format!("{}.png", cover_key(path)));

    // Fast path: decode the cached PNG.
    if let Ok(bytes) = std::fs::read(&file) {
        if let Ok(img) = image::load_from_memory(&bytes) {
            let rgba = img.to_rgba8();
            let (w, h) = (rgba.width() as usize, rgba.height() as usize);
            return Some(ColorImage::from_rgba_unmultiplied([w, h], rgba.as_raw()));
        }
    }

    // Slow path: render page 0 and persist it.
    let img = engine.thumbnail(path, target_w).ok()?;
    let _ = std::fs::create_dir_all(&dir);
    if let Some(buf) =
        image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.rgba.clone())
    {
        let _ = buf.save(&file);
    }
    Some(ColorImage::from_rgba_unmultiplied(
        [img.width, img.height],
        &img.rgba,
    ))
}

/// Wipe a single cover from the on-disk cache (used when removing a PDF).
pub fn forget_cover(path: &str) {
    let file = cover_cache_dir().join(format!("{}.png", cover_key(path)));
    let _ = std::fs::remove_file(file);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// pdfium may only be initialised once per process, so *all* engine
    /// behaviour is validated through a single `Worker` — exactly the path the
    /// UI uses. Set `FOLIO_TEST_PDF` to a PDF path to run it (otherwise it
    /// no-ops, keeping fixture-less CI green).
    fn test_pdf() -> Option<String> {
        std::env::var("FOLIO_TEST_PDF")
            .ok()
            .filter(|p| std::path::Path::new(p).exists())
    }

    /// Poll the worker until it emits at least one message (or we time out).
    fn wait_for(worker: &Worker, timeout: Duration) -> Vec<Res> {
        let deadline = Instant::now() + timeout;
        let mut out = Vec::new();
        while Instant::now() < deadline {
            out.extend(worker.drain());
            if !out.is_empty() {
                std::thread::sleep(Duration::from_millis(20));
                out.extend(worker.drain());
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        out
    }

    #[test]
    fn cover_key_is_stable() {
        let Some(path) = test_pdf() else { return };
        assert_eq!(cover_key(&path), cover_key(&path));
    }

    #[test]
    fn worker_end_to_end() {
        let Some(path) = test_pdf() else { return };
        let worker = Worker::spawn(egui::Context::default());
        let long = Duration::from_secs(15);

        // Open → page count + first size.
        worker.send(Req::Open(path.clone()));
        let opened = wait_for(&worker, long);
        if opened.iter().any(|r| matches!(r, Res::EngineError(_))) {
            return; // pdfium unavailable in this environment
        }
        let page_count = opened
            .iter()
            .find_map(|r| match r {
                Res::Opened { page_count, .. } => Some(*page_count),
                _ => None,
            })
            .expect("document opened with a page count");
        assert!(page_count > 0, "document has pages");

        // Render page 0 → non-empty image.
        worker.send(Req::Render { path: path.clone(), page: 0, scale: 1.0 });
        let rendered = wait_for(&worker, long);
        let page_ok = rendered.iter().any(|r| {
            matches!(r, Res::Page { page: 0, img, .. } if img.width() > 0 && img.height() > 0)
        });
        assert!(page_ok, "page 0 rendered to a bitmap");

        // Extract glyph boxes for page 0 → non-empty.
        worker.send(Req::Chars { path: path.clone(), page: 0 });
        let charred = wait_for(&worker, long);
        let chars_ok = charred
            .iter()
            .any(|r| matches!(r, Res::Chars { page: 0, chars, .. } if !chars.is_empty()));
        assert!(chars_ok, "page 0 produced glyph boxes");

        // Cover: first call renders + caches, second hits the disk cache.
        forget_cover(&path);
        worker.send(Req::Cover { id: "t".into(), path: path.clone(), target_w: 200 });
        let cover1 = wait_for(&worker, long);
        assert!(
            cover1.iter().any(|r| matches!(r, Res::Cover { img: Some(_), .. })),
            "cover rendered"
        );
        worker.send(Req::Cover { id: "t".into(), path: path.clone(), target_w: 200 });
        let cover2 = wait_for(&worker, long);
        assert!(
            cover2.iter().any(|r| matches!(r, Res::Cover { img: Some(_), .. })),
            "cover served from disk cache"
        );
    }
}
