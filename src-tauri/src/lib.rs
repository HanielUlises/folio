use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::PathBuf};
use tauri::{
    http::{Request, Response},
    AppHandle, Manager, Runtime, UriSchemeContext,
};


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Topic {
    pub id:    String,
    pub name:  String,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfEntry {
    pub id:      String,
    pub path:    String,
    pub name:    String,
    pub size:    u64,
    pub added:   u64,
    #[serde(rename = "topicId")]
    pub topic_id: Option<String>,
    pub exists:  bool,
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
    pub id:    String,
    pub page:  u32,
    pub rects: Vec<HighlightRect>,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppData {
    pub topics:     Vec<Topic>,
    pub pdfs:       Vec<PdfEntry>,
    pub highlights: HashMap<String, Vec<PdfHighlight>>,
}

#[derive(Debug, Serialize)]
pub struct OpenedFile {
    pub path:  String,
    pub name:  String,
    pub size:  u64,
    pub added: u64,
}

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

#[tauri::command]
fn get_data(app: AppHandle) -> AppData {
    load_data(&app)
}

#[tauri::command]
fn save_data(app: AppHandle, data: AppData) -> Result<bool, String> {
    save_data_inner(&app, &data)?;
    Ok(true)
}

#[tauri::command]
fn check_exists(path: String) -> bool {
    std::path::Path::new(&path).exists()
}

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
                let pb = std::path::Path::new(&path_str);
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

#[tauri::command]
fn get_folio_url(path: String) -> String {
    let encoded = URL_SAFE_NO_PAD.encode(path.as_bytes());
    format!("folio://local/{}", encoded)
}
pub fn handle_folio_protocol<R: Runtime>(_ctx: UriSchemeContext<'_, R>, request: Request<Vec<u8>>) -> Response<Vec<u8>> {    let uri = request.uri();
    let encoded = uri.path().trim_start_matches('/');

    let file_path = match URL_SAFE_NO_PAD.decode(encoded) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(s)  => s,
            Err(e) => return error_response(400, &e.to_string()),
        },
        Err(e) => return error_response(400, &e.to_string()),
    };

    let path = std::path::Path::new(&file_path);
    if !path.exists() {
        return error_response(404, "not found");
    }

    let total = match fs::metadata(path) {
        Ok(m)  => m.len(),
        Err(e) => return error_response(500, &e.to_string()),
    };

    let range_header = request
        .headers()
        .get("range")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    if let Some(range) = range_header {
        // Parse "bytes=start-end"
        let (start, end) = parse_range(&range, total);
        let len = (end - start + 1) as usize;

        let data = match read_range(path, start, len) {
            Ok(d)  => d,
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
            Ok(d)  => d,
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

fn read_range(path: &std::path::Path, start: u64, len: usize) -> Result<Vec<u8>, String> {
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .register_uri_scheme_protocol("folio", handle_folio_protocol)
        .invoke_handler(tauri::generate_handler![
            get_data,
            save_data,
            check_exists,
            open_pdf_dialog,
            get_folio_url,
        ])
        .run(tauri::generate_context!())
        .expect("error running Folio");
}
