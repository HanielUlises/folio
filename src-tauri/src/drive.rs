//! Optional Google Drive integration (`drive` feature).
//!
//! Implements the OAuth 2.0 "installed app" flow with PKCE and a loopback
//! redirect, persists the tokens, and exposes a small Drive v3 client. All
//! networking is blocking (`ureq`) and is only ever run on a background thread,
//! never on the UI thread — the UI talks to it through [`DriveState`].

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SCOPE: &str = "https://www.googleapis.com/auth/drive.readonly";
const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const ABOUT_URL: &str = "https://www.googleapis.com/drive/v3/about?fields=user";

// ── Paths ────────────────────────────────────────────────────────────────────

fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("com.folio.pdf")
}
fn creds_path() -> PathBuf { data_dir().join("drive-credentials.json") }
fn tokens_path() -> PathBuf { data_dir().join("drive-tokens.json") }
/// Where downloaded Drive PDFs are cached for offline reading.
pub fn cache_dir() -> PathBuf { data_dir().join("drive-cache") }

// ── Credentials & tokens ─────────────────────────────────────────────────────

#[derive(Clone, Deserialize)]
pub struct Credentials {
    pub client_id: String,
    pub client_secret: String,
}

impl Credentials {
    /// Load from `$FOLIO_GOOGLE_CLIENT_ID` / `$FOLIO_GOOGLE_CLIENT_SECRET`, else
    /// from `drive-credentials.json` in the application data directory. The file
    /// lives outside the repository and is never committed.
    pub fn load() -> Option<Self> {
        if let (Ok(id), Ok(secret)) = (
            std::env::var("FOLIO_GOOGLE_CLIENT_ID"),
            std::env::var("FOLIO_GOOGLE_CLIENT_SECRET"),
        ) {
            return Some(Self { client_id: id, client_secret: secret });
        }
        serde_json::from_str(&std::fs::read_to_string(creds_path()).ok()?).ok()
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Tokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub expiry: u64, // unix seconds
}

impl Tokens {
    fn load() -> Option<Self> {
        serde_json::from_str(&std::fs::read_to_string(tokens_path()).ok()?).ok()
    }
    fn save(&self) {
        if let Some(p) = tokens_path().parent() {
            let _ = std::fs::create_dir_all(p);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(tokens_path(), s);
        }
    }
    fn fresh(&self) -> bool {
        !self.access_token.is_empty() && now() < self.expiry.saturating_sub(60)
    }
}

// ── OAuth flow ───────────────────────────────────────────────────────────────

/// Run the full interactive consent flow and return freshly minted tokens.
/// Opens the system browser and waits for the loopback redirect.
pub fn connect(creds: &Credentials) -> Result<Tokens, String> {
    // PKCE verifier/challenge.
    let mut raw = [0u8; 48];
    getrandom::getrandom(&mut raw).map_err(|e| e.to_string())?;
    let verifier = b64url(&raw);
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));

    // Loopback listener on an ephemeral port.
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}");

    let auth = format!(
        "{AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&scope={}\
         &code_challenge={}&code_challenge_method=S256&access_type=offline&prompt=consent",
        enc(&creds.client_id), enc(&redirect), enc(SCOPE), challenge,
    );
    open_browser(&auth);

    let code = wait_for_code(&listener)?;

    let body = post_form(TOKEN_URL, &[
        ("client_id", &creds.client_id),
        ("client_secret", &creds.client_secret),
        ("code", &code),
        ("code_verifier", &verifier),
        ("grant_type", "authorization_code"),
        ("redirect_uri", &redirect),
    ])?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let tokens = Tokens {
        access_token: v["access_token"].as_str().ok_or("no access_token in response")?.to_string(),
        refresh_token: v["refresh_token"].as_str().unwrap_or_default().to_string(),
        expiry: now() + v["expires_in"].as_u64().unwrap_or(3600),
    };
    tokens.save();
    Ok(tokens)
}

/// Exchange a refresh token for a new access token.
pub fn refresh(creds: &Credentials, refresh_token: &str) -> Result<Tokens, String> {
    let body = post_form(TOKEN_URL, &[
        ("client_id", &creds.client_id),
        ("client_secret", &creds.client_secret),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ])?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let tokens = Tokens {
        access_token: v["access_token"].as_str().ok_or("no access_token in refresh")?.to_string(),
        refresh_token: refresh_token.to_string(),
        expiry: now() + v["expires_in"].as_u64().unwrap_or(3600),
    };
    tokens.save();
    Ok(tokens)
}

/// Ensure a usable access token, refreshing if the current one has expired.
fn ensure(creds: &Credentials, tokens: &Tokens) -> Result<Tokens, String> {
    if tokens.fresh() {
        Ok(tokens.clone())
    } else if !tokens.refresh_token.is_empty() {
        refresh(creds, &tokens.refresh_token)
    } else {
        Err("session expired; reconnect".into())
    }
}

// ── Drive v3 client ──────────────────────────────────────────────────────────

/// The signed-in account's email address (confirms the connection).
pub fn account_email(access_token: &str) -> Result<String, String> {
    let body = get_auth(ABOUT_URL, access_token)?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    Ok(v["user"]["emailAddress"].as_str().unwrap_or("unknown").to_string())
}

#[derive(Clone)]
pub struct DriveFile {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub modified: String,
}

/// List the user's PDF files (most-recently-modified first).
pub fn list_pdfs(access_token: &str) -> Result<Vec<DriveFile>, String> {
    let url = "https://www.googleapis.com/drive/v3/files\
        ?q=mimeType%3D%27application%2Fpdf%27%20and%20trashed%3Dfalse\
        &fields=files(id,name,size,modifiedTime)&pageSize=200&orderBy=modifiedTime%20desc";
    let body = get_auth(url, access_token)?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let files = v["files"].as_array().map(|a| {
        a.iter().map(|f| DriveFile {
            id: f["id"].as_str().unwrap_or_default().to_string(),
            name: f["name"].as_str().unwrap_or_default().to_string(),
            size: f["size"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0),
            modified: f["modifiedTime"].as_str().unwrap_or_default().to_string(),
        }).collect()
    }).unwrap_or_default();
    Ok(files)
}

/// Download a file's bytes to `dest`, creating parent directories as needed.
pub fn download(access_token: &str, id: &str, dest: &Path) -> Result<(), String> {
    let url = format!("https://www.googleapis.com/drive/v3/files/{id}?alt=media");
    match ureq::get(&url).set("Authorization", &format!("Bearer {access_token}")).call() {
        Ok(r) => {
            let mut buf = Vec::new();
            r.into_reader().read_to_end(&mut buf).map_err(|e| e.to_string())?;
            if let Some(p) = dest.parent() {
                let _ = std::fs::create_dir_all(p);
            }
            std::fs::write(dest, buf).map_err(|e| e.to_string())
        }
        Err(ureq::Error::Status(c, r)) => Err(format!("HTTP {c}: {}", r.into_string().unwrap_or_default())),
        Err(e) => Err(e.to_string()),
    }
}

// ── UI-facing async state ────────────────────────────────────────────────────

pub enum Status {
    Disconnected,
    Connecting,
    Connected(String), // account email
    Error(String),
}

/// Result of a background job the UI is waiting on.
enum Job {
    Connected(Tokens, String),
    Listed(Vec<DriveFile>),
    Failed(String),
}

pub struct DriveState {
    pub status: Status,
    creds: Option<Credentials>,
    tokens: Option<Tokens>,
    rx: Option<Receiver<Job>>,
    pub files: Vec<DriveFile>,
    pub browsing: bool,
}

impl DriveState {
    pub fn new() -> Self {
        let creds = Credentials::load();
        let tokens = Tokens::load();
        let status = if creds.is_none() {
            Status::Error("no drive-credentials.json".into())
        } else if tokens.is_some() {
            Status::Connected("saved session".into())
        } else {
            Status::Disconnected
        };
        Self { status, creds, tokens, rx: None, files: Vec::new(), browsing: false }
    }

    pub fn is_busy(&self) -> bool {
        matches!(self.status, Status::Connecting) || self.rx.is_some()
    }

    /// Start the interactive connect flow on a background thread.
    pub fn connect(&mut self) {
        let Some(creds) = self.creds.clone() else {
            self.status = Status::Error("no credentials configured".into());
            return;
        };
        self.status = Status::Connecting;
        let (tx, rx) = std::sync::mpsc::channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            let job = match connect(&creds).and_then(|t| account_email(&t.access_token).map(|e| (t, e))) {
                Ok((t, email)) => Job::Connected(t, email),
                Err(e) => Job::Failed(e),
            };
            let _ = tx.send(job);
        });
    }

    /// Fetch the user's PDFs on a background thread.
    pub fn browse(&mut self) {
        let (Some(creds), Some(tokens)) = (self.creds.clone(), self.tokens.clone()) else {
            self.status = Status::Error("connect first".into());
            return;
        };
        self.browsing = true;
        let (tx, rx) = std::sync::mpsc::channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            let job = match ensure(&creds, &tokens).and_then(|t| list_pdfs(&t.access_token)) {
                Ok(files) => Job::Listed(files),
                Err(e) => Job::Failed(e),
            };
            let _ = tx.send(job);
        });
    }

    /// Download a chosen file to the cache and return its local path.
    pub fn fetch_to_cache(&self, file: &DriveFile) -> Result<PathBuf, String> {
        let (creds, tokens) = (
            self.creds.as_ref().ok_or("no credentials")?,
            self.tokens.as_ref().ok_or("not connected")?,
        );
        let tok = ensure(creds, tokens)?;
        let dest = cache_dir().join(format!("{}.pdf", file.id));
        if !dest.exists() {
            download(&tok.access_token, &file.id, &dest)?;
        }
        Ok(dest)
    }

    /// Poll for completed background work; call once per frame.
    pub fn poll(&mut self) {
        let Some(rx) = &self.rx else { return };
        if let Ok(job) = rx.try_recv() {
            self.rx = None;
            match job {
                Job::Connected(t, email) => {
                    self.tokens = Some(t);
                    self.status = Status::Connected(email);
                }
                Job::Listed(files) => self.files = files,
                Job::Failed(e) => self.status = Status::Error(e),
            }
        }
    }
}

// ── HTTP / encoding helpers ──────────────────────────────────────────────────

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn post_form(url: &str, fields: &[(&str, &str)]) -> Result<String, String> {
    match ureq::post(url).send_form(fields) {
        Ok(r) => r.into_string().map_err(|e| e.to_string()),
        Err(ureq::Error::Status(c, r)) => Err(format!("HTTP {c}: {}", r.into_string().unwrap_or_default())),
        Err(e) => Err(e.to_string()),
    }
}
fn get_auth(url: &str, token: &str) -> Result<String, String> {
    match ureq::get(url).set("Authorization", &format!("Bearer {token}")).call() {
        Ok(r) => r.into_string().map_err(|e| e.to_string()),
        Err(ureq::Error::Status(c, r)) => Err(format!("HTTP {c}: {}", r.into_string().unwrap_or_default())),
        Err(e) => Err(e.to_string()),
    }
}

fn open_browser(url: &str) {
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

/// Block until the browser hits the loopback redirect, then return the `code`.
fn wait_for_code(listener: &TcpListener) -> Result<String, String> {
    let (mut stream, _) = listener.accept().map_err(|e| e.to_string())?;
    let mut buf = [0u8; 8192];
    let n = stream.read(&mut buf).map_err(|e| e.to_string())?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let path = req.lines().next().unwrap_or("").split_whitespace().nth(1).unwrap_or("");
    let query = path.splitn(2, '?').nth(1).unwrap_or("");

    let (mut code, mut err) = (None, None);
    for kv in query.split('&') {
        let mut it = kv.splitn(2, '=');
        match (it.next(), it.next()) {
            (Some("code"), Some(v)) => code = Some(dec(v)),
            (Some("error"), Some(v)) => err = Some(dec(v)),
            _ => {}
        }
    }

    let ok = code.is_some();
    let msg = if ok {
        "Folio is now connected to Google Drive. You can close this tab."
    } else {
        "Authorization failed. You can close this tab and try again."
    };
    let html = format!(
        "<html><body style='font-family:sans-serif;text-align:center;margin-top:4em'>\
         <h2>{msg}</h2></body></html>"
    );
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
        html.len()
    );
    code.ok_or_else(|| err.unwrap_or_else(|| "no authorization code returned".into()))
}

/// Percent-encode for query components (RFC 3986 unreserved set kept literal).
fn enc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(b as char),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}
/// Minimal percent-decoding for the redirect query.
fn dec(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(h) => { out.push(h); i += 3; }
                Err(_) => { out.push(b'%'); i += 1; }
            },
            b'+' => { out.push(b' '); i += 1; }
            c => { out.push(c); i += 1; }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
