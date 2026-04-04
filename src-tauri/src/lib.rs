// ============================================================================
//  Folio — lib.rs
//  Tauri v2 backend with pdfium-render as the PDF engine.
//
//  Architecture:
//    • PdfEngine    — thread-safe wrapper around pdfium-render's Pdfium handle
//    • PdfSession   — one open document per tab, stored in a DashMap
//    • Tauri cmds   — one async fn per frontend invoke()
//
//  All PDF work (load, render, text extraction, thumbnail, highlight) is
//  done in Rust. The frontend receives PNG bytes as base64 strings and
//  pre-computed text spans with bounding boxes.
// ============================================================================

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use image::{ImageBuffer, Rgba};
use once_cell::sync::OnceCell;
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tauri::{
    http::{Request, Response},
    AppHandle, Emitter, Manager, Runtime, UriSchemeContext,
};
use uuid::Uuid;

// ─── Global pdfium instance ───────────────────────────────────────────────────
//
// pdfium-render's Pdfium struct is NOT Send by default; the "thread_safe"
// feature enables this. We wrap it in Arc<Mutex<>> so all Tauri commands
// (which run on different threads) can share one loaded library.
//
// We intentionally use OnceCell (not lazy_static) so any initialisation
// error is surfaced at startup, not silently swallowed.
static PDFIUM: OnceCell<Arc<Mutex<Pdfium>>> = OnceCell::new();

fn pdfium() -> Arc<Mutex<Pdfium>> {
    PDFIUM.get().expect("pdfium not initialised").clone()
}

/// Initialise pdfium-render. Tries bundled binary first, then system path.
fn init_pdfium(app: &AppHandle) {
    // The pdfium-render crate can download / bundle the shared library.
    // At runtime we look for it next to the executable (bundled) or fall
    // back to whatever is on LD_LIBRARY_PATH / system path.
    let bindings = {
        // Try to find the bundled pdfium next to our binary.
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()));

        // Also check the Tauri resource dir (where we copy it during bundling).
        let res_dir = app
            .path()
            .resource_dir()
            .ok()
            .map(|d| d.join("pdfium"));

        let candidates: Vec<PathBuf> = [exe_dir, res_dir]
            .into_iter()
            .flatten()
            .collect();

        let mut result = None;
        for dir in &candidates {
            if let Ok(b) = Pdfium::bind_to_library(
                Pdfium::pdfium_platform_library_name_at_path(dir),
            ) {
                result = Some(b);
                break;
            }
        }

        // Final fallback: system pdfium / whatever pdfium-render finds
        result.unwrap_or_else(|| {
            Pdfium::bind_to_system_library()
                .expect(
                    "pdfium shared library not found. \
                     Place libpdfium.so (Linux), pdfium.dll (Windows), or \
                     libpdfium.dylib (macOS) next to the Folio binary, or \
                     install it system-wide.",
                )
        })
    };

    let instance = Arc::new(Mutex::new(Pdfium::new(bindings)));
    PDFIUM.set(instance).expect("pdfium already initialised");
}

// ─── Open document sessions ───────────────────────────────────────────────────
//
// Each reader tab gets a unique session_id. We keep the loaded PdfDocument
// in memory so rendering individual pages is fast (no re-parse).
//
// PdfDocument is NOT Send, so we store raw bytes and re-open on demand, OR
// we open the document inside a Mutex. We choose: store the path and re-open
// in each command with the Mutex held for the duration. This is safe because
// all commands are async but we hold the mutex only briefly.
//
// For large documents this is fast because pdfium memory-maps the file.

#[derive(Debug, Clone)]
struct Session {
    path: PathBuf,
    page_count: u32,
}

// Global session map: session_id → Session
type SessionMap = Arc<Mutex<HashMap<String, Session>>>;

static SESSIONS: OnceCell<SessionMap> = OnceCell::new();

fn sessions() -> SessionMap {
    SESSIONS.get().expect("sessions not initialised").clone()
}

// ─── Data model (same as before, kept compatible) ─────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Topic {
    pub id: String,
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfEntry {
    pub id: String,
    pub path: String,
    pub name: String,
    pub size: u64,
    pub added: u64,
    #[serde(rename = "topicId")]
    pub topic_id: Option<String>,
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HighlightRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfHighlight {
    pub id: String,
    pub page: u32,
    pub rects: Vec<HighlightRect>,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppData {
    pub topics: Vec<Topic>,
    pub pdfs: Vec<PdfEntry>,
    pub highlights: HashMap<String, Vec<PdfHighlight>>,
}

#[derive(Debug, Serialize)]
pub struct OpenedFile {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub added: u64,
}

/// Bounding box for a single text span on a page (CSS-friendly, top-left origin)
#[derive(Debug, Serialize)]
pub struct TextSpan {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// Font size in page units (unscaled)
    pub font_size: f64,
}

/// Info returned after opening a document
#[derive(Debug, Serialize)]
pub struct DocInfo {
    pub session_id: String,
    pub page_count: u32,
}

// ─── Persistence ──────────────────────────────────────────────────────────────

fn data_file(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .expect("no app data dir")
        .join("folio-data.json")
}

fn load_data(app: &AppHandle) -> AppData {
    let path = data_file(app);
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_data_inner(app: &AppHandle, data: &AppData) -> Result<(), String> {
    let path = data_file(app);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())
}

// ─── Tauri commands ───────────────────────────────────────────────────────────

/// Load app state from disk
#[tauri::command]
fn get_data(app: AppHandle) -> AppData {
    load_data(&app)
}

/// Persist app state to disk
#[tauri::command]
fn save_data(app: AppHandle, data: AppData) -> Result<bool, String> {
    save_data_inner(&app, &data)?;
    Ok(true)
}

/// Check whether a file path exists on disk
#[tauri::command]
fn check_exists(path: String) -> bool {
    Path::new(&path).exists()
}

/// Open the system file picker and return selected PDF paths
#[tauri::command]
async fn open_pdf_dialog(app: AppHandle) -> Vec<OpenedFile> {
    use tauri_plugin_dialog::DialogExt;

    let paths = app
        .dialog()
        .file()
        .set_title("Add PDFs")
        .add_filter("PDF Files", &["pdf"])
        .blocking_pick_files();

    match paths {
        Some(file_paths) => file_paths
            .into_iter()
            .filter_map(|fp| {
                let path_str = fp.to_string();
                let pb = Path::new(&path_str);
                let meta = fs::metadata(pb).ok()?;
                let name = pb
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                let added = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                Some(OpenedFile {
                    path: path_str,
                    name,
                    size: meta.len(),
                    added,
                })
            })
            .collect(),
        None => vec![],
    }
}

// ─── PDF session management ───────────────────────────────────────────────────

/// Open a PDF file and create a session.  Returns session_id + page_count.
/// Called once per tab when the reader is first shown.
#[tauri::command]
async fn pdf_open(path: String) -> Result<DocInfo, String> {
    let pb = PathBuf::from(&path);
    if !pb.exists() {
        return Err(format!("File not found: {}", path));
    }

    let pdfium_arc = pdfium();
    let pdfium = pdfium_arc.lock().map_err(|e| e.to_string())?;

    let doc = pdfium
        .load_pdf_from_file(&pb, None)
        .map_err(|e| format!("pdfium load error: {:?}", e))?;

    let page_count = doc.pages().len() as u32;
    drop(doc); // release pdfium lock before touching session map

    let session_id = Uuid::new_v4().to_string();
    sessions()
        .lock()
        .map_err(|e| e.to_string())?
        .insert(session_id.clone(), Session { path: pb, page_count });

    Ok(DocInfo { session_id, page_count })
}

/// Close a session (called when a reader tab is closed)
#[tauri::command]
async fn pdf_close(session_id: String) -> Result<(), String> {
    sessions()
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&session_id);
    Ok(())
}

/// Render a single page to a PNG, returned as a base64 string.
///
/// `scale` is the CSS device-pixel-ratio-aware zoom factor:
///   rendered_width_px = page_width_points * scale
///
/// We render at `scale * device_pixel_ratio` for crisp HiDPI output and let
/// the frontend display at CSS `scale` width.  For simplicity we treat dpr=1
/// here; the frontend can pass `scale * window.devicePixelRatio`.
#[tauri::command]
async fn pdf_render_page(
    session_id: String,
    page_index: u32, // 0-based
    scale: f64,
) -> Result<String, String> {
    let path = {
        let map = sessions().lock().map_err(|e| e.to_string())?;
        map.get(&session_id)
            .ok_or_else(|| format!("session not found: {}", session_id))?
            .path
            .clone()
    };

    // Clamp scale to sane range to prevent OOM on huge zoom
    let scale = scale.clamp(0.1, 6.0);

    let pdfium_arc = pdfium();
    let pdfium = pdfium_arc.lock().map_err(|e| e.to_string())?;

    let doc = pdfium
        .load_pdf_from_file(&path, None)
        .map_err(|e| format!("pdfium: {:?}", e))?;

    let pages = doc.pages();
    let page = pages
        .get(page_index as u16)
        .map_err(|e| format!("page {}: {:?}", page_index, e))?;

    let w_pts = page.width().value;
    let h_pts = page.height().value;

    let w_px = ((w_pts as f64) * scale).ceil() as u32;
    let h_px = ((h_pts as f64) * scale).ceil() as u32;

    // Render to RGBA bitmap via pdfium-render's image feature
    let bitmap = page
        .render_with_config(
            &PdfRenderConfig::new()
                .set_target_width(w_px as i32)
                .set_target_height(h_px as i32)
                .set_reverse_byte_order(false), // BGRA → RGBA with swap below
        )
        .map_err(|e| format!("render: {:?}", e))?;

    // pdfium-render returns a DynamicImage when the "image" feature is enabled.
    // Convert to PNG bytes.
    let dyn_img = bitmap
        .as_image()
        .into_rgba8();

    // pdfium renders with BGRA byte order; swap B and R channels.
    let mut rgba = dyn_img;
    for pixel in rgba.pixels_mut() {
        let [b, g, r, a] = pixel.0;
        pixel.0 = [r, g, b, a];
    }

    // White background compositing (PDF pages are transparent over white in viewers)
    let mut composited: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_pixel(w_px, h_px, Rgba([255u8, 255, 255, 255]));
    for (x, y, src) in rgba.enumerate_pixels() {
        let dst = composited.get_pixel_mut(x, y);
        let [sr, sg, sb, sa] = src.0;
        if sa == 255 {
            *dst = Rgba([sr, sg, sb, 255]);
        } else if sa > 0 {
            let a = sa as f32 / 255.0;
            let blend = |s: u8, d: u8| -> u8 {
                ((s as f32) * a + (d as f32) * (1.0 - a)) as u8
            };
            *dst = Rgba([blend(sr, 255), blend(sg, 255), blend(sb, 255), 255]);
        }
    }

    let mut png_bytes: Vec<u8> = Vec::new();
    composited
        .write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            image::ImageFormat::Png,
        )
        .map_err(|e| e.to_string())?;

    Ok(B64.encode(&png_bytes))
}

/// Return page dimensions in points (unscaled) for layout calculations.
#[tauri::command]
async fn pdf_page_size(
    session_id: String,
    page_index: u32,
) -> Result<(f64, f64), String> {
    let path = {
        let map = sessions().lock().map_err(|e| e.to_string())?;
        map.get(&session_id)
            .ok_or_else(|| "session not found".to_string())?
            .path
            .clone()
    };

    let pdfium_arc = pdfium();
    let pdfium = pdfium_arc.lock().map_err(|e| e.to_string())?;

    let doc = pdfium
        .load_pdf_from_file(&path, None)
        .map_err(|e| format!("{:?}", e))?;
    let page = doc
        .pages()
        .get(page_index as u16)
        .map_err(|e| format!("{:?}", e))?;

    Ok((page.width().value as f64, page.height().value as f64))
}

/// Extract all text spans from a page with their bounding boxes.
///
/// Returns a Vec of TextSpan, each with (x, y, w, h) in scaled pixels
/// (multiply by `scale` to match the rendered image).  The y axis is
/// CSS-style (top = 0, increasing downward) — pdfium uses bottom-up, so
/// we flip here.
#[tauri::command]
async fn pdf_text_layer(
    session_id: String,
    page_index: u32,
    scale: f64,
) -> Result<Vec<TextSpan>, String> {
    let path = {
        let map = sessions().lock().map_err(|e| e.to_string())?;
        map.get(&session_id)
            .ok_or_else(|| "session not found".to_string())?
            .path
            .clone()
    };

    let pdfium_arc = pdfium();
    let pdfium = pdfium_arc.lock().map_err(|e| e.to_string())?;

    let doc = pdfium
        .load_pdf_from_file(&path, None)
        .map_err(|e| format!("{:?}", e))?;
    let page = doc
        .pages()
        .get(page_index as u16)
        .map_err(|e| format!("{:?}", e))?;

    let page_height = page.height().value as f64;
    let scale = scale.clamp(0.1, 6.0);

    let text_page = page
        .text()
        .map_err(|e| format!("text: {:?}", e))?;

    let mut spans: Vec<TextSpan> = Vec::new();

    for char_index in 0..text_page.len() {
        // Get each character and its bounds
        let ch = match text_page.get(char_index) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let text = ch.unicode_string();
        if text.trim().is_empty() {
            // Accumulate whitespace into previous span
            if let Some(last) = spans.last_mut() {
                last.text.push(' ');
            }
            continue;
        }

        let bounds = ch.bounds().map_err(|e| format!("bounds: {:?}", e))?;

        // pdfium bounding box: bottom-left origin
        let bl_x = bounds.left.value as f64;
        let bl_y = bounds.bottom.value as f64;
        let w    = (bounds.right.value - bounds.left.value) as f64;
        let h    = (bounds.top.value   - bounds.bottom.value) as f64;

        // Convert to CSS top-left origin
        let css_x = bl_x * scale;
        let css_y = (page_height - bl_y - h) * scale;
        let css_w = w * scale;
        let css_h = h * scale;

        let font_size = h; // approximate

        // Try to merge with previous span if vertically aligned and close
        let merged = spans.last_mut().and_then(|last| {
            let same_line = (last.y - css_y).abs() < last.h * 0.5;
            let adjacent  = css_x - (last.x + last.w) < last.h * 0.8;
            if same_line && adjacent {
                last.text.push_str(&text);
                last.w = css_x + css_w - last.x;
                last.h = last.h.max(css_h);
                Some(())
            } else {
                None
            }
        });

        if merged.is_none() {
            spans.push(TextSpan {
                text,
                x: css_x,
                y: css_y,
                w: css_w,
                h: css_h,
                font_size,
            });
        }
    }

    Ok(spans)
}

/// Render a thumbnail of page 0 for the library shelf.
/// Returns base64 PNG. Width is fixed at `thumb_width` px.
#[tauri::command]
async fn pdf_thumbnail(path: String, thumb_width: u32) -> Result<String, String> {
    let pb = PathBuf::from(&path);
    if !pb.exists() {
        return Err("file not found".to_string());
    }

    let thumb_width = thumb_width.clamp(60, 600);

    let pdfium_arc = pdfium();
    let pdfium = pdfium_arc.lock().map_err(|e| e.to_string())?;

    let doc = pdfium
        .load_pdf_from_file(&pb, None)
        .map_err(|e| format!("{:?}", e))?;

    let pages = doc.pages();
    let page = pages.get(0).map_err(|e| format!("{:?}", e))?;

    let w_pts = page.width().value;
    let scale = thumb_width as f32 / w_pts;
    let h_px = ((page.height().value) * scale).ceil() as i32;

    let bitmap = page
        .render_with_config(
            &PdfRenderConfig::new()
                .set_target_width(thumb_width as i32)
                .set_target_height(h_px),
        )
        .map_err(|e| format!("{:?}", e))?;

    let mut rgba = bitmap.as_image().into_rgba8();
    for pixel in rgba.pixels_mut() {
        let [b, g, r, a] = pixel.0;
        pixel.0 = [r, g, b, a];
    }

    // Composite over white
    let mut composited: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_pixel(thumb_width, h_px as u32, Rgba([255u8, 255, 255, 255]));
    for (x, y, src) in rgba.enumerate_pixels() {
        if x < composited.width() && y < composited.height() {
            let dst = composited.get_pixel_mut(x, y);
            let [sr, sg, sb, sa] = src.0;
            if sa == 255 {
                *dst = Rgba([sr, sg, sb, 255]);
            } else if sa > 0 {
                let a = sa as f32 / 255.0;
                let blend = |s: u8, d: u8| -> u8 {
                    ((s as f32) * a + (d as f32) * (1.0 - a)) as u8
                };
                *dst = Rgba([blend(sr, 255), blend(sg, 255), blend(sb, 255), 255]);
            }
        }
    }

    let mut png: Vec<u8> = Vec::new();
    composited
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Jpeg)
        .map_err(|e| e.to_string())?;

    Ok(format!("data:image/jpeg;base64,{}", B64.encode(&png)))
}

/// Full-text search within a document.  Returns list of (page_index, text_excerpt).
#[tauri::command]
async fn pdf_search(
    session_id: String,
    query: String,
) -> Result<Vec<(u32, String)>, String> {
    if query.trim().is_empty() {
        return Ok(vec![]);
    }

    let (path, page_count) = {
        let map = sessions().lock().map_err(|e| e.to_string())?;
        let s = map
            .get(&session_id)
            .ok_or_else(|| "session not found".to_string())?;
        (s.path.clone(), s.page_count)
    };

    let ql = query.to_lowercase();
    let mut results: Vec<(u32, String)> = Vec::new();

    let pdfium_arc = pdfium();
    let pdfium = pdfium_arc.lock().map_err(|e| e.to_string())?;

    let doc = pdfium
        .load_pdf_from_file(&path, None)
        .map_err(|e| format!("{:?}", e))?;

    for pg_idx in 0..page_count {
        let page = match doc.pages().get(pg_idx as u16) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let text_page = match page.text() {
            Ok(tp) => tp,
            Err(_) => continue,
        };

        // Collect all text on the page
        let mut full_text = String::new();
        for i in 0..text_page.len() {
            if let Ok(ch) = text_page.get(i) {
                full_text.push_str(&ch.unicode_string());
            }
        }

        let full_lower = full_text.to_lowercase();
        if full_lower.contains(&ql) {
            // Build a short excerpt around the first match
            let pos = full_lower.find(&ql).unwrap_or(0);
            let start = pos.saturating_sub(40);
            let end   = (pos + ql.len() + 40).min(full_text.len());
            let excerpt = format!(
                "…{}…",
                full_text[start..end].trim()
            );
            results.push((pg_idx, excerpt));
        }
    }

    Ok(results)
}

// ─── folio:// custom protocol (kept for compatibility / optional use) ─────────

pub fn handle_folio_protocol<R: Runtime>(
    _ctx: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let uri = request.uri();
    let encoded = uri.path().trim_start_matches('/');

    let file_path = match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(encoded) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => return error_response(400, &e.to_string()),
        },
        Err(e) => return error_response(400, &e.to_string()),
    };

    let path = Path::new(&file_path);
    if !path.exists() {
        return error_response(404, "not found");
    }

    let total = match fs::metadata(path) {
        Ok(m) => m.len(),
        Err(e) => return error_response(500, &e.to_string()),
    };

    let range_header = request
        .headers()
        .get("range")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    if let Some(range) = range_header {
        let (start, end) = parse_range(&range, total);
        let len = (end - start + 1) as usize;
        let data = match read_range(path, start, len) {
            Ok(d) => d,
            Err(e) => return error_response(500, &e.to_string()),
        };
        Response::builder()
            .status(206)
            .header("Content-Type", "application/pdf")
            .header("Content-Range", format!("bytes {}-{}/{}", start, end, total))
            .header("Content-Length", len.to_string())
            .header("Accept-Ranges", "bytes")
            .header("Access-Control-Allow-Origin", "*")
            .body(data)
            .unwrap()
    } else {
        let data = match fs::read(path) {
            Ok(d) => d,
            Err(e) => return error_response(500, &e.to_string()),
        };
        Response::builder()
            .status(200)
            .header("Content-Type", "application/pdf")
            .header("Content-Length", total.to_string())
            .header("Accept-Ranges", "bytes")
            .header("Access-Control-Allow-Origin", "*")
            .body(data)
            .unwrap()
    }
}

fn parse_range(range: &str, total: u64) -> (u64, u64) {
    let bytes_part = range.trim_start_matches("bytes=");
    let mut parts = bytes_part.splitn(2, '-');
    let start: u64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let end: u64 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(total - 1)
        .min(total - 1);
    (start, end)
}

fn read_range(path: &Path, start: u64, len: usize) -> Result<Vec<u8>, String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = fs::File::open(path).map_err(|e| e.to_string())?;
    f.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

fn error_response(status: u16, msg: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/plain")
        .body(msg.as_bytes().to_vec())
        .unwrap()
}

// ─── Drag-and-drop file handler ───────────────────────────────────────────────
//
// Tauri v2 emits a "tauri://drag-drop" event with paths[].
// We listen in the builder and re-emit as our own "folio-drop" event so the
// frontend receives cleaned file info without using pdf.js.

fn setup_drag_drop<R: Runtime>(app: &tauri::App<R>) {
    let handle = app.handle().clone();
    app.listen("tauri://drag-drop", move |event| {
        // The payload is {"paths": [...], "position": {...}}
        #[derive(Deserialize)]
        struct DragPayload {
            paths: Vec<String>,
        }
        if let Ok(payload) = serde_json::from_str::<DragPayload>(event.payload()) {
            let pdfs: Vec<OpenedFile> = payload
                .paths
                .into_iter()
                .filter(|p| p.to_lowercase().ends_with(".pdf"))
                .filter_map(|p| {
                    let pb = Path::new(&p);
                    let meta = fs::metadata(pb).ok()?;
                    let name = pb
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let added = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    Some(OpenedFile {
                        path: p,
                        name,
                        size: meta.len(),
                        added,
                    })
                })
                .collect();

            if !pdfs.is_empty() {
                let _ = handle.emit("folio-drop", pdfs);
            }
        }
    });
}

// ─── App entry point ──────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .register_uri_scheme_protocol("folio", handle_folio_protocol)
        .setup(|app| {
            // Initialise pdfium before any command can use it
            init_pdfium(app.handle());

            // Initialise session map
            SESSIONS
                .set(Arc::new(Mutex::new(HashMap::new())))
                .expect("sessions already initialised");

            // Wire up drag-and-drop
            setup_drag_drop(app);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Data persistence
            get_data,
            save_data,
            check_exists,
            open_pdf_dialog,
            // PDF engine
            pdf_open,
            pdf_close,
            pdf_render_page,
            pdf_page_size,
            pdf_text_layer,
            pdf_thumbnail,
            pdf_search,
        ])
        .run(tauri::generate_context!())
        .expect("error running Folio");
}
