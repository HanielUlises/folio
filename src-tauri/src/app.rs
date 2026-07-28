//! Folio — native egui application.

use crate::engine::{self, Req, Res, Worker};
use crate::icon::{self, Icon};
use crate::model::*;
use crate::pdf::{CharBox, OutlineItem};
use eframe::egui::{self, Color32, ColorImage, CornerRadius, Pos2, Rect, Sense, Stroke, TextureHandle, Vec2};
use std::collections::{HashMap, HashSet};

// ── Palette ─────────────────────────────────────────────────────────────────
// All surface colours live in a `Pal` so the whole UI can be re-themed at once.
// The warm gold accent and the blue selection ribbon are shared by both themes;
// everything else flips between the dark and light variants below.
#[derive(Clone, Copy)]
struct Pal {
    bg: Color32,
    panel: Color32,
    card: Color32,
    card_hov: Color32,
    border: Color32,
    accent: Color32,
    txt: Color32,
    txt_dim: Color32,
    page_bg: Color32,
    // Live text-selection ribbon (transient), distinct from saved highlight
    // annotations. A translucent Breeze-style selection blue (#3daee9),
    // premultiplied — reads like a native desktop selection, text stays legible.
    select_fill: Color32,
}

impl Pal {
    const fn dark() -> Self {
        Self {
            bg: Color32::from_rgb(0x14, 0x14, 0x16),
            panel: Color32::from_rgb(0x1a, 0x1a, 0x1d),
            card: Color32::from_rgb(0x20, 0x20, 0x24),
            card_hov: Color32::from_rgb(0x2a, 0x2a, 0x30),
            border: Color32::from_rgb(0x33, 0x33, 0x3a),
            accent: Color32::from_rgb(0xd4, 0xa8, 0x43),
            txt: Color32::from_rgb(0xee, 0xeb, 0xe5),
            txt_dim: Color32::from_rgb(0x8e, 0x8b, 0x86),
            page_bg: Color32::from_rgb(0x21, 0x21, 0x25),
            select_fill: Color32::from_rgba_premultiplied(0x19, 0x48, 0x60, 0x69),
        }
    }

    /// True light theme: near-white surfaces, neutral greys.
    const fn light() -> Self {
        Self {
            bg: Color32::from_rgb(0xf6, 0xf7, 0xf9),
            panel: Color32::from_rgb(0xed, 0xef, 0xf2),
            card: Color32::from_rgb(0xff, 0xff, 0xff),
            card_hov: Color32::from_rgb(0xe9, 0xec, 0xf1),
            border: Color32::from_rgb(0xd7, 0xda, 0xe0),
            accent: Color32::from_rgb(0xb8, 0x8a, 0x2a),
            txt: Color32::from_rgb(0x1c, 0x1e, 0x22),
            txt_dim: Color32::from_rgb(0x6b, 0x70, 0x78),
            page_bg: Color32::from_rgb(0xe4, 0xe6, 0xea),
            select_fill: Color32::from_rgba_premultiplied(0x2c, 0x6f, 0x99, 0x55),
        }
    }

    /// Warm sepia theme: easy-on-the-eyes paper tone.
    const fn sepia() -> Self {
        Self {
            bg: Color32::from_rgb(0xf4, 0xf2, 0xee),
            panel: Color32::from_rgb(0xea, 0xe7, 0xe1),
            card: Color32::from_rgb(0xff, 0xff, 0xfb),
            card_hov: Color32::from_rgb(0xf0, 0xec, 0xe4),
            border: Color32::from_rgb(0xd6, 0xd1, 0xc8),
            accent: Color32::from_rgb(0xb8, 0x8a, 0x2a),
            txt: Color32::from_rgb(0x33, 0x2c, 0x22),
            txt_dim: Color32::from_rgb(0x78, 0x70, 0x62),
            page_bg: Color32::from_rgb(0xe7, 0xe1, 0xd4),
            select_fill: Color32::from_rgba_premultiplied(0x2c, 0x6f, 0x99, 0x55),
        }
    }

    fn from_name(name: &str) -> Self {
        match name {
            "light" => Self::light(),
            "sepia" => Self::sepia(),
            _ => Self::dark(),
        }
    }

    /// Whether a theme name wants egui's dark base visuals.
    fn is_dark(name: &str) -> bool {
        !matches!(name, "light" | "sepia")
    }
}

const COVER_W: u32 = 300;

#[derive(Clone, PartialEq)]
enum Filter {
    All,
    Unsorted,
    Topic(String),
    Tag(String),
}

enum View {
    Library,
    Reader,
}

/// The active pointer modality in the reader. Following the ISOTYPE idea, each
/// mode is a single clear pictogram rather than a hidden modifier: the hand
/// drags the page, the I-beam selects text, the marker selects and highlights.
#[derive(Clone, Copy, PartialEq)]
enum ReaderTool {
    Pan,       // drag to move the page; no text selection
    Select,    // drag to select text
    Highlight, // drag to select text and highlight it in one gesture
}

/// How the "All PDFs" view arranges cards.
#[derive(Clone, Copy, PartialEq)]
enum LibLayout {
    Grouped, // sectioned by topic, ISOTYPE-style folders
    Flat,    // every PDF in one grid
}

enum Cover {
    Loading,
    Ready(TextureHandle),
    Failed,
}

/// One text line on a page: a contiguous run of glyphs (in reading order)
/// sharing a baseline and height. `end` is exclusive. Built once from `chars`
/// and used for all hit-testing, mirroring how Poppler/Okular group glyphs.
#[derive(Clone, Copy)]
struct LineBox {
    start: usize,
    end: usize,
    top: f32,
    bottom: f32,
    left: f32,
    right: f32,
}

struct PageSlot {
    size_pts: Option<(f32, f32)>,
    texture: Option<TextureHandle>,
    rendered_zoom: f32,
    requested_zoom: f32,
    chars: Option<Vec<CharBox>>,
    lines: Option<Vec<LineBox>>,
    chars_requested: bool,
}

impl Default for PageSlot {
    fn default() -> Self {
        Self {
            size_pts: None,
            texture: None,
            rendered_zoom: -1.0,
            requested_zoom: -1.0,
            chars: None,
            lines: None,
            chars_requested: false,
        }
    }
}

struct Selection {
    page: u32,
    /// Anchor and focus caret indices (0..=char_count). The selected glyphs are
    /// those in the half-open range `[min(start,end), max(start,end))`.
    start: usize,
    end: usize,
    text: String,
}

struct Reader {
    pdf_id: String,
    path: String,
    name: String,
    page_count: u32, // 0 until the worker reports the document is open
    default_size: (f32, f32),
    zoom: f32, // pixels per point
    pages: Vec<PageSlot>,
    current_page: u32,
    hl_color: String,
    selection: Option<Selection>,
    open_failed: Option<String>,
    scroll_offset: f32, // last vertical scroll offset, for zoom-to-cursor
    /// Highlight awaiting a delete confirmation: (highlight id, screen anchor).
    pending_delete: Option<(String, Pos2)>,
    outline: Vec<OutlineItem>,   // table of contents
    scroll_to_page: Option<u32>, // pending jump target (1-based)
    jump_input: String,          // page-number box contents
    tool: ReaderTool,            // active pointer modality
    collapsed: HashSet<usize>,   // contents: collapsed chapter indices
}

pub struct Folio {
    worker: Worker,
    engine_err: Option<String>,
    data: AppData,
    pal: Pal,
    view: View,
    filter: Filter,
    search: String,
    covers: HashMap<String, Cover>,
    reader: Option<Reader>,

    // simple modal state
    show_new_topic: bool,
    show_new_tag: bool,
    edit_name: String,
    edit_color: String,
    /// When creating a topic from a PDF's menu, assign it to this PDF on create.
    pending_topic_pdf: Option<String>,
    toast: Option<(String, f64)>,
    // library layout customization
    card_w: f32,
    show_sidebar: bool,
    show_outline: bool, // reader: table-of-contents panel
    lib_layout: LibLayout,
    show_topics: bool, // sidebar: topics section expanded
    show_tags: bool,   // sidebar: tags section expanded
    drag_topic: Option<usize>, // sidebar: index of the topic being dragged to reorder
    drag_pdf: Option<String>,  // library: id of the PDF card being dragged to reorder
    confirm_delete_topic: Option<String>, // topic id awaiting delete confirmation
    #[cfg(feature = "drive")]
    drive: crate::drive::DriveState,
}

impl Folio {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut data = AppData::load();
        if data.theme.is_empty() {
            data.theme = "dark".to_string();
        }
        let pal = Pal::from_name(&data.theme);
        setup_style(&cc.egui_ctx, &pal, Pal::is_dark(&data.theme));
        let worker = Worker::spawn(cc.egui_ctx.clone());
        // verify existence for the "missing" badge
        for p in &mut data.pdfs {
            p.exists = std::path::Path::new(&p.path).exists();
        }
        Self {
            worker,
            engine_err: None,
            data,
            pal,
            view: View::Library,
            filter: Filter::All,
            search: String::new(),
            covers: HashMap::new(),
            reader: None,
            show_new_topic: false,
            show_new_tag: false,
            edit_name: String::new(),
            edit_color: PALETTE[0].to_string(),
            pending_topic_pdf: None,
            toast: None,
            card_w: 156.0,
            show_sidebar: true,
            show_outline: true,
            lib_layout: LibLayout::Grouped,
            show_topics: true,
            show_tags: true,
            drag_topic: None,
            drag_pdf: None,
            confirm_delete_topic: None,
            #[cfg(feature = "drive")]
            drive: crate::drive::DriveState::new(),
        }
    }

    /// Switch to a named theme ("dark" | "light" | "sepia") and persist it.
    fn set_theme(&mut self, ctx: &egui::Context, name: &str) {
        if self.data.theme == name {
            return;
        }
        self.data.theme = name.to_string();
        self.pal = Pal::from_name(name);
        setup_style(ctx, &self.pal, Pal::is_dark(name));
        self.save();
    }

    fn save(&self) {
        self.data.save();
    }

    fn toast(&mut self, ctx: &egui::Context, msg: &str) {
        self.toast = Some((msg.to_string(), ctx.input(|i| i.time) + 2.2));
    }

    /// Google Drive browser window (feature = "drive"). Lists the account's
    /// PDFs; opening one downloads it to the cache and hands the local path to
    /// the reader, keyed by the Drive file id so highlights persist.
    #[cfg(feature = "drive")]
    fn ui_drive(&mut self, ctx: &egui::Context) {
        use crate::drive::{DriveFile, Status};
        let pal = self.pal;
        if !self.drive.browsing {
            return;
        }
        let mut open_file: Option<DriveFile> = None;
        let mut save_file: Option<DriveFile> = None;
        let (mut refresh, mut close, mut disconnect, mut switch) = (false, false, false, false);
        egui::Window::new("drive-picker")
            .title_bar(false)
            .collapsible(false)
            .resizable(true)
            .default_size([520.0, 460.0])
            .min_width(380.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(egui::Frame::new().fill(pal.panel).stroke(Stroke::new(1.0_f32, pal.border)).corner_radius(CornerRadius::same(12)).inner_margin(egui::Margin::same(16)))
            .show(ctx, |ui| {
                // ── Header: logo + title + account ────────────────────────────
                ui.horizontal(|ui| {
                    let (lr, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
                    icon::draw_drive(ui.painter(), lr, true);
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Google Drive").color(pal.txt).size(15.0).strong());
                        let sub = match &self.drive.status {
                            Status::Connected(email) => format!("Signed in as {email}"),
                            _ => "Select a PDF to open".to_string(),
                        };
                        ui.label(egui::RichText::new(sub).color(pal.txt_dim).size(11.0));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let acct = icon::button(ui, Icon::Account, 24.0, pal.txt_dim, pal.card_hov).on_hover_text("Account");
                        let popup_id = ui.make_persistent_id("drive-account-menu");
                        if acct.clicked() {
                            ui.memory_mut(|m| m.toggle_popup(popup_id));
                        }
                        egui::popup_below_widget(ui, popup_id, &acct, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                            ui.set_min_width(160.0);
                            if ui.button("Switch account…").clicked() {
                                switch = true;
                            }
                            if ui.button("Disconnect").clicked() {
                                disconnect = true;
                            }
                        });
                    });
                });
                if let Status::Error(e) = &self.drive.status {
                    ui.add_space(4.0);
                    ui.colored_label(Color32::from_rgb(0xd9, 0x65, 0x65), egui::RichText::new(e.clone()).size(11.0));
                }
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(6.0);

                // ── Body: recents + full document list ────────────────────────
                let mut act = |a: RowAct, f: &crate::drive::DriveFile| match a {
                    RowAct::Open => open_file = Some(f.clone()),
                    RowAct::Save => save_file = Some(f.clone()),
                    RowAct::None => {}
                };
                if self.drive.is_busy() {
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| {
                        ui.spinner();
                        ui.label(egui::RichText::new("Loading your PDFs…").color(pal.txt_dim).size(12.0));
                    });
                } else {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(340.0).show(ui, |ui| {
                        // Quick access: recently opened documents.
                        if !self.drive.recents.is_empty() {
                            ui.label(egui::RichText::new("RECENT").color(pal.txt_dim).size(10.0).strong());
                            ui.add_space(2.0);
                            for f in &self.drive.recents {
                                act(drive_row(ui, pal, f), f);
                            }
                            ui.add_space(6.0);
                            ui.separator();
                            ui.add_space(4.0);
                        }
                        if self.drive.files.is_empty() {
                            ui.add_space(16.0);
                            ui.vertical_centered(|ui| {
                                ui.label(egui::RichText::new("Browse hasn't loaded — press Refresh.").color(pal.txt_dim).size(12.0));
                            });
                        } else {
                            ui.label(egui::RichText::new(format!("ALL · {} document(s)", self.drive.files.len())).color(pal.txt_dim).size(10.0).strong());
                            ui.add_space(2.0);
                            for f in &self.drive.files {
                                act(drive_row(ui, pal, f), f);
                            }
                        }
                    });
                }

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.add_enabled(!self.drive.is_busy(), egui::Button::new("Refresh")).clicked() {
                        refresh = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            close = true;
                        }
                    });
                });
            });

        if disconnect {
            self.drive.disconnect();
            self.toast(ctx, "Disconnected from Google Drive");
        } else if switch {
            // Forget the current account and re-run consent (with the account
            // chooser); the picker closes and reopens via the top-bar logo.
            self.drive.disconnect();
            self.drive.connect();
        } else if refresh {
            self.drive.browse();
        }
        if let Some(f) = open_file {
            // Download in the background so the loading animation can spin.
            self.drive.start_download(f);
        }
        if let Some(f) = save_file {
            // Save a Drive PDF to a location the user chooses on this device.
            if let Some(dest) = rfd::FileDialog::new().set_file_name(&f.name).add_filter("PDF", &["pdf"]).save_file() {
                self.toast(ctx, "Saving…");
                match self.drive.fetch_to_cache(&f) {
                    Ok(cache) => match std::fs::copy(&cache, &dest) {
                        Ok(_) => {
                            self.drive.note_opened(&f);
                            self.toast(ctx, "Saved to device");
                        }
                        Err(e) => self.drive.status = Status::Error(e.to_string()),
                    },
                    Err(e) => self.drive.status = Status::Error(e),
                }
            }
        }
        if close {
            self.drive.browsing = false;
        }
    }

    fn filtered_pdfs(&self) -> Vec<PdfEntry> {
        let q = self.search.to_lowercase();
        self.data
            .pdfs
            .iter()
            .filter(|p| match &self.filter {
                Filter::All => true,
                Filter::Unsorted => p.topic_ids.is_empty(),
                Filter::Topic(id) => p.in_topic(id),
                Filter::Tag(id) => p.tag_ids.iter().any(|t| t == id),
            })
            .filter(|p| q.is_empty() || p.name.to_lowercase().contains(&q))
            .cloned()
            .collect()
    }

    fn add_pdfs(&mut self, ctx: &egui::Context, paths: Vec<std::path::PathBuf>) {
        let mut added = 0;
        let mut dup = 0;
        for path in paths {
            let ps = path.to_string_lossy().to_string();
            if self.data.pdfs.iter().any(|p| p.path == ps) {
                dup += 1;
                continue;
            }
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("untitled")
                .to_string();
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let added_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let topic_ids = match &self.filter {
                Filter::Topic(id) => vec![id.clone()],
                _ => vec![],
            };
            let tag_ids = match &self.filter {
                Filter::Tag(id) => vec![id.clone()],
                _ => vec![],
            };
            self.data.pdfs.push(PdfEntry {
                id: uuid::Uuid::new_v4().to_string(),
                path: ps,
                name,
                size,
                added: added_ms,
                topic_ids,
                legacy_topic_id: None,
                tag_ids,
                exists: true,
            });
            added += 1;
        }
        if added > 0 {
            self.save();
        }
        let msg = match (added, dup) {
            (0, 0) => None,
            (0, d) => Some(format!("Already in library ({d} skipped)")),
            (a, 0) => Some(format!("Added {a} PDF{}", if a == 1 { "" } else { "s" })),
            (a, d) => Some(format!("Added {a}, {d} already in library")),
        };
        if let Some(msg) = msg {
            self.toast(ctx, &msg);
        }
    }

    fn open_reader(&mut self, pdf: &PdfEntry) {
        if !pdf.exists {
            self.engine_err = Some(format!("File not found: {}", pdf.path));
            return;
        }
        self.open_reader_path(pdf.id.clone(), pdf.path.clone(), pdf.name.clone());
    }

    /// Open the reader on an explicit local path. `id` keys the document's
    /// highlights, so a stable id (e.g. a Drive file id) keeps them across runs.
    fn open_reader_path(&mut self, id: String, path: String, name: String) {
        // Ask the worker to (re)open the document; pages/chars stream in later.
        self.worker.send(Req::Close);
        self.worker.send(Req::Open(path.clone()));
        self.reader = Some(Reader {
            pdf_id: id,
            path,
            name,
            page_count: 0,
            default_size: (595.0, 842.0),
            zoom: 1.4,
            pages: Vec::new(),
            current_page: 1,
            hl_color: HL_COLORS[0].to_string(),
            selection: None,
            open_failed: None,
            scroll_offset: 0.0,
            pending_delete: None,
            outline: Vec::new(),
            scroll_to_page: None,
            jump_input: String::new(),
            tool: ReaderTool::Pan,
            collapsed: HashSet::new(),
        });
        self.view = View::Reader;
    }

    // ── Worker results ───────────────────────────────────────────────────────
    fn handle_res(&mut self, ctx: &egui::Context, res: Res) {
        match res {
            Res::EngineError(e) => {
                self.engine_err = Some(e);
                // Nothing can render now — resolve any pending covers to a badge.
                for c in self.covers.values_mut() {
                    if matches!(c, Cover::Loading) {
                        *c = Cover::Failed;
                    }
                }
            }
            Res::Opened { path, page_count, first_size } => {
                if let Some(r) = &mut self.reader {
                    if r.path == path {
                        r.page_count = page_count;
                        r.default_size = first_size;
                        r.pages = (0..page_count).map(|_| PageSlot::default()).collect();
                    }
                }
            }
            Res::Outline { path, items } => {
                if let Some(r) = &mut self.reader {
                    if r.path == path {
                        r.outline = items;
                    }
                }
            }
            Res::OpenFailed { path, err } => {
                if let Some(r) = &mut self.reader {
                    if r.path == path {
                        r.open_failed = Some(err);
                    }
                }
            }
            Res::Page { path, page, scale, size_pts, img } => {
                if let Some(r) = &mut self.reader {
                    if r.path == path {
                        if let Some(slot) = r.pages.get_mut(page as usize) {
                            let tex = upload(ctx, &format!("pg-{}-{}", r.pdf_id, page), img);
                            slot.texture = Some(tex);
                            slot.rendered_zoom = scale;
                            slot.size_pts = Some(size_pts);
                        }
                    }
                }
            }
            Res::Chars { path, page, chars } => {
                if let Some(r) = &mut self.reader {
                    if r.path == path {
                        if let Some(slot) = r.pages.get_mut(page as usize) {
                            slot.chars = Some(chars);
                        }
                    }
                }
            }
            Res::Cover { id, img } => {
                let cover = match img {
                    Some(color) => Cover::Ready(upload(ctx, &format!("cover-{id}"), color)),
                    None => Cover::Failed,
                };
                self.covers.insert(id, cover);
            }
        }
    }
}

impl eframe::App for Folio {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let pal = self.pal;
        // absorb everything the worker produced since the last frame
        for res in self.worker.drain() {
            self.handle_res(ctx, res);
        }

        #[cfg(feature = "drive")]
        {
            self.drive.poll();
            // Keep repainting while a background Drive job is in flight so its
            // result (connect / list / account name / download) lands smoothly.
            if self.drive.wants_repaint() {
                ctx.request_repaint_after(std::time::Duration::from_millis(80));
            }
            // A finished Drive download opens in the reader.
            if let Some((f, path)) = self.drive.just_downloaded.take() {
                self.drive.note_opened(&f);
                self.open_reader_path(f.id.clone(), path.to_string_lossy().into_owned(), f.name.clone());
            }
            self.ui_drive(ctx);
            // Loading animation while a document downloads.
            if let Some(name) = self.drive.downloading.clone() {
                egui::Area::new(egui::Id::new("drive-loading"))
                    .order(egui::Order::Foreground)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        egui::Frame::new()
                            .fill(pal.panel)
                            .stroke(Stroke::new(1.0_f32, pal.border))
                            .corner_radius(CornerRadius::same(12))
                            .inner_margin(egui::Margin::symmetric(24, 20))
                            .show(ui, |ui| {
                                ui.vertical_centered(|ui| {
                                    ui.spinner();
                                    ui.add_space(8.0);
                                    ui.label(egui::RichText::new("Loading from Drive").color(pal.txt).size(13.0).strong());
                                    ui.label(egui::RichText::new(elide(&name, 40)).color(pal.txt_dim).size(11.0));
                                });
                            });
                    });
            }
        }

        // Drag & drop: import any PDF files dropped onto the window.
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .filter(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("pdf")))
                .collect()
        });
        if !dropped.is_empty() {
            if matches!(self.view, View::Reader) {
                self.view = View::Library;
            }
            self.add_pdfs(ctx, dropped);
        }
        // While files hover over the window, show a drop hint.
        let hovering_files = ctx.input(|i| !i.raw.hovered_files.is_empty());

        // top bar
        let mut pick_theme: Option<&str> = None;
        egui::TopBottomPanel::top("topbar")
            .exact_height(44.0)
            .frame(egui::Frame::new().fill(pal.panel).inner_margin(egui::Margin::symmetric(14, 0)))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(egui::RichText::new("Folio").color(pal.accent).size(18.0).strong());
                    ui.add_space(8.0);
                    match &self.view {
                        View::Library => {
                            ui.label(egui::RichText::new("· Library").color(pal.txt_dim).size(13.0));
                        }
                        View::Reader => {
                            if ui.button("‹  Library").clicked() {
                                self.view = View::Library;
                            }
                            if let Some(r) = &self.reader {
                                ui.add_space(6.0);
                                ui.label(egui::RichText::new(&r.name).color(pal.txt_dim).size(13.0));
                            }
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let cur = self.data.theme.as_str();
                        let cur_label = match cur {
                            "light" => "Light",
                            "sepia" => "Sepia",
                            _ => "Dark",
                        };
                        ui.menu_button(egui::RichText::new(format!("Theme: {cur_label}")).size(12.0), |ui| {
                            for (name, label) in [("dark", "Dark"), ("light", "Light"), ("sepia", "Sepia")] {
                                if ui.selectable_label(cur == name, label).clicked() {
                                    pick_theme = Some(name);
                                    ui.close_menu();
                                }
                            }
                        });
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new("v0.8.7 · native").color(pal.txt_dim).size(11.0));

                        // ── Google Drive sync control ─────────────────────────
                        #[cfg(feature = "drive")]
                        {
                            use crate::drive::Status;
                            ui.add_space(12.0);
                            let connected = matches!(self.drive.status, Status::Connected(_));
                            let busy = self.drive.is_busy();
                            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
                            if resp.hovered() {
                                ui.painter().rect_filled(rect, CornerRadius::same(6), pal.card_hov);
                            }
                            // Full colour when synced, grey otherwise.
                            icon::draw_drive(ui.painter(), rect.shrink(5.0), connected && !busy);
                            let tip = match &self.drive.status {
                                Status::Connected(who) => format!("Google Drive · {who}\nClick to browse documents"),
                                Status::Connecting => "Connecting to Google Drive…\nClick to cancel".to_string(),
                                Status::Error(e) => format!("Google Drive not connected\n{e}\nClick to connect"),
                                Status::Disconnected => "Connect Google Drive".to_string(),
                            };
                            let resp = resp.on_hover_text(tip);
                            // Always accept a click: a stuck attempt (no redirect
                            // ever arrived) must never lock the control forever.
                            if resp.clicked() {
                                if matches!(self.drive.status, Status::Connecting) {
                                    self.drive.cancel();
                                } else if connected {
                                    self.drive.browse();
                                } else if self.drive.needs_credentials() {
                                    let hint = crate::drive::credentials_hint();
                                    self.toast(ctx, &format!("Google Drive not set up — add drive-credentials.json at {hint}"));
                                } else {
                                    self.drive.connect();
                                }
                            }
                        }
                    });
                });
            });
        if let Some(name) = pick_theme {
            self.set_theme(ctx, name);
        }

        match self.view {
            View::Library => self.ui_library(ctx),
            View::Reader => self.ui_reader(ctx),
        }

        // modals
        self.ui_modals(ctx);

        // toast
        if let Some((msg, until)) = self.toast.clone() {
            if ctx.input(|i| i.time) < until {
                egui::Area::new(egui::Id::new("toast"))
                    .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -20.0])
                    .show(ctx, |ui| {
                        egui::Frame::new()
                            .fill(pal.card)
                            .stroke(Stroke::new(1.0_f32, pal.border))
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(egui::Margin::symmetric(14, 8))
                            .show(ui, |ui| {
                                ui.label(egui::RichText::new(msg).color(pal.txt));
                            });
                    });
                ctx.request_repaint();
            } else {
                self.toast = None;
            }
        }

        // drop hint while dragging files over the window
        if hovering_files {
            let screen = ctx.screen_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop-hint")));
            painter.rect_filled(screen, CornerRadius::ZERO, pal.accent.gamma_multiply(0.10));
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                "Drop PDFs to add them",
                egui::FontId::proportional(22.0),
                pal.accent,
            );
        }
    }
}

impl Folio {
    // ── Library ─────────────────────────────────────────────────────────────
    fn ui_library(&mut self, ctx: &egui::Context) {
        let pal = self.pal;
        // sidebar (resizable + collapsible)
        if self.show_sidebar {
            egui::SidePanel::left("sidebar")
                .default_width(238.0)
                .width_range(180.0..=420.0)
                .resizable(true)
                .frame(egui::Frame::new().fill(pal.panel).inner_margin(egui::Margin::same(0)))
                .show(ctx, |ui| self.ui_sidebar(ui));
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(pal.bg).inner_margin(egui::Margin::same(16)))
            .show(ctx, |ui| {
                if let Some(err) = &self.engine_err {
                    ui.colored_label(Color32::from_rgb(0xd9, 0x65, 0x65), err);
                }
                // toolbar
                ui.horizontal(|ui| {
                    // collapse / expand the sidebar
                    if icon::button(ui, Icon::Sidebar, 26.0, pal.txt, pal.card_hov).on_hover_text("Show / hide sidebar").clicked() {
                        self.show_sidebar = !self.show_sidebar;
                    }
                    ui.add_space(2.0);
                    let (title, title_icon) = match &self.filter {
                        Filter::All => ("All PDFs".to_string(), Some(Icon::Library)),
                        Filter::Unsorted => ("Unsorted".to_string(), Some(Icon::Inbox)),
                        Filter::Topic(id) => (self.data.topic(id).map(|t| t.name.clone()).unwrap_or_default(), Some(Icon::Folder)),
                        Filter::Tag(id) => (self.data.tag(id).map(|t| t.name.clone()).unwrap_or_default(), Some(Icon::Tag)),
                    };
                    if let Some(ic) = title_icon {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                        icon::draw(ui.painter(), ic, r, pal.txt);
                    }
                    ui.heading(egui::RichText::new(title).color(pal.txt));

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new(egui::RichText::new("+  Add PDFs").color(Color32::BLACK)).fill(pal.accent)).clicked() {
                            if let Some(files) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_files() {
                                self.add_pdfs(ctx, files);
                            }
                        }
                        ui.add_space(10.0);
                        // search with a magnifier
                        ui.add(egui::TextEdit::singleline(&mut self.search).desired_width(150.0).hint_text(egui::RichText::new("Search…").color(pal.txt_dim.gamma_multiply(0.55))));
                        let (sr, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                        icon::draw(ui.painter(), Icon::Search, sr, pal.txt_dim);
                        ui.add_space(10.0);
                        ui.separator();
                        // segmented cover-size control
                        ui.horizontal(|ui| {
                            for (lbl, wv) in [("S", 128.0f32), ("M", 168.0), ("L", 212.0)] {
                                let sel = (self.card_w - wv).abs() < 1.0;
                                if ui.selectable_label(sel, lbl).clicked() {
                                    self.card_w = wv;
                                }
                            }
                        });
                        // view mode toggle (only meaningful for All PDFs)
                        if matches!(self.filter, Filter::All) {
                            ui.separator();
                            if icon::button(ui, Icon::Groups, 26.0, if self.lib_layout == LibLayout::Grouped { pal.accent } else { pal.txt_dim }, pal.card_hov).on_hover_text("Grouped by topic").clicked() {
                                self.lib_layout = LibLayout::Grouped;
                            }
                            if icon::button(ui, Icon::Grid, 26.0, if self.lib_layout == LibLayout::Flat { pal.accent } else { pal.txt_dim }, pal.card_hov).on_hover_text("All in one grid").clicked() {
                                self.lib_layout = LibLayout::Flat;
                            }
                        }
                    });
                });
                ui.add_space(10.0);

                let pdfs = self.filtered_pdfs();
                if pdfs.is_empty() {
                    ui.add_space(60.0);
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("No PDFs here yet").color(pal.txt_dim).size(15.0));
                        ui.label(egui::RichText::new("Click “Add PDFs” to get started").color(pal.txt_dim).size(12.0));
                    });
                    return;
                }

                let grouped = matches!(self.filter, Filter::All) && self.lib_layout == LibLayout::Grouped;
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if grouped {
                        self.ui_grouped(ui, ctx, &pdfs);
                    } else {
                        self.ui_grid(ui, ctx, &pdfs, "all");
                    }
                });
                // Finished dragging a card → persist the new order.
                if self.drag_pdf.is_some() && ui.input(|i| i.pointer.any_released()) {
                    self.drag_pdf = None;
                    self.save();
                }
            });
    }

    fn ui_sidebar(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal;
        ui.add_space(10.0);
        egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 0)).show(ui, |ui| {
            // filter rows
            let all_n = self.data.pdfs.len();
            self.sidebar_row(ui, "All PDFs", Icon::Library, pal.txt_dim, all_n, matches!(self.filter, Filter::All), Filter::All, false);
            let uns = self.data.pdfs.iter().filter(|p| p.topic_ids.is_empty()).count();
            if uns > 0 {
                self.sidebar_row(ui, "Unsorted", Icon::Inbox, pal.txt_dim, uns, matches!(self.filter, Filter::Unsorted), Filter::Unsorted, false);
            }

            ui.add_space(10.0);
            let (toggle, add) = self.section_header(ui, "TOPICS", self.show_topics);
            if toggle { self.show_topics = !self.show_topics; }
            if add {
                self.show_new_topic = true;
                self.edit_name.clear();
                self.edit_color = PALETTE[0].to_string();
            }
            if self.show_topics {
                let topics = self.data.topics.clone();
                let pointer = ui.input(|i| i.pointer.hover_pos());
                let mut row_rects: Vec<(usize, Rect)> = Vec::new();
                for (i, t) in topics.iter().enumerate() {
                    let n = self.data.pdfs.iter().filter(|p| p.in_topic(&t.id)).count();
                    let active = matches!(&self.filter, Filter::Topic(id) if id == &t.id);
                    let resp = self.sidebar_row(ui, &t.name, Icon::Folder, parse_hex(&t.color), n, active, Filter::Topic(t.id.clone()), true);
                    row_rects.push((i, resp.rect));
                    if self.drag_topic == Some(i) {
                        // The row being dragged: lifted outline + grab cursor.
                        ui.painter().rect_stroke(resp.rect, CornerRadius::same(6), Stroke::new(1.5_f32, pal.accent), egui::StrokeKind::Middle);
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    }
                    if resp.drag_started() {
                        self.drag_topic = Some(i);
                    }
                    // Right-click: recolour or delete the topic.
                    resp.context_menu(|ui| {
                        ui.label(egui::RichText::new("Colour").color(pal.txt_dim).size(10.0).strong());
                        ui.horizontal(|ui| {
                            for c in PALETTE {
                                let (r, cr) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
                                ui.painter().circle_filled(r.center(), 7.0, parse_hex(c));
                                if &t.color == c {
                                    ui.painter().circle_stroke(r.center(), 8.0, Stroke::new(2.0_f32, pal.txt));
                                }
                                if cr.clicked() {
                                    if let Some(topic) = self.data.topics.iter_mut().find(|x| x.id == t.id) {
                                        topic.color = c.to_string();
                                    }
                                    self.save();
                                    ui.close_menu();
                                }
                            }
                        });
                        ui.separator();
                        if ui.button("Delete topic").clicked() {
                            self.confirm_delete_topic = Some(t.id.clone());
                            ui.close_menu();
                        }
                    });
                }
                // Live reorder: while dragging, moving over another row shifts it.
                if let (Some(from), Some(p)) = (self.drag_topic, pointer) {
                    if let Some(&(to, _)) = row_rects.iter().find(|(_, r)| r.contains(p)) {
                        if to != from {
                            let item = self.data.topics.remove(from);
                            self.data.topics.insert(to, item);
                            self.drag_topic = Some(to);
                        }
                    }
                }
                // Drop: persist the new order.
                if self.drag_topic.is_some() && ui.input(|i| i.pointer.any_released()) {
                    self.drag_topic = None;
                    self.save();
                }
                if topics.is_empty() {
                    ui.label(egui::RichText::new("No topics yet").color(pal.txt_dim).size(11.0));
                }
            }

            ui.add_space(10.0);
            let (toggle, add) = self.section_header(ui, "TAGS", self.show_tags);
            if toggle { self.show_tags = !self.show_tags; }
            if add {
                self.show_new_tag = true;
                self.edit_name.clear();
                self.edit_color = PALETTE[1].to_string();
            }
            if self.show_tags {
                let tags = self.data.tags.clone();
                ui.horizontal_wrapped(|ui| {
                    for t in &tags {
                        let active = matches!(&self.filter, Filter::Tag(id) if id == &t.id);
                        let col = parse_hex(&t.color);
                        let txt = egui::RichText::new(format!("● {}", t.name)).color(if active { col } else { pal.txt_dim }).size(12.0);
                        let resp = ui.add(egui::Button::new(txt).fill(if active { pal.card_hov } else { pal.card }).stroke(Stroke::new(1.0_f32, if active { col } else { pal.border })));
                        if resp.clicked() {
                            self.filter = if active { Filter::All } else { Filter::Tag(t.id.clone()) };
                        }
                        resp.context_menu(|ui| {
                            if ui.button("Delete tag").clicked() {
                                self.data.pdfs.iter_mut().for_each(|p| p.tag_ids.retain(|x| x != &t.id));
                                self.data.tags.retain(|x| x.id != t.id);
                                if matches!(&self.filter, Filter::Tag(id) if id == &t.id) { self.filter = Filter::All; }
                                self.save();
                                ui.close_menu();
                            }
                        });
                    }
                    if tags.is_empty() {
                        ui.label(egui::RichText::new("No tags yet").color(pal.txt_dim).size(11.0));
                    }
                });
            }
        });

        // footer with export/import
        egui::TopBottomPanel::bottom("sb-foot")
            .frame(egui::Frame::new().fill(pal.panel).inner_margin(egui::Margin::same(10)))
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Export").clicked() {
                        if let Some(path) = rfd::FileDialog::new().set_file_name("folio-library.json").add_filter("JSON", &["json"]).save_file() {
                            if let Ok(js) = serde_json::to_string_pretty(&self.data) {
                                let _ = std::fs::write(&path, js);
                            }
                        }
                    }
                    if ui.button("Import").clicked() {
                        if let Some(path) = rfd::FileDialog::new().add_filter("JSON", &["json"]).pick_file() {
                            if let Ok(txt) = std::fs::read_to_string(&path) {
                                if let Ok(incoming) = serde_json::from_str::<AppData>(&txt) {
                                    merge_into(&mut self.data, incoming);
                                    self.save();
                                }
                            }
                        }
                    }
                });
                ui.label(egui::RichText::new(format!("{} PDFs · {} topics · {} tags", self.data.pdfs.len(), self.data.topics.len(), self.data.tags.len())).color(pal.txt_dim).size(11.0));
            });
    }

    fn sidebar_row(&mut self, ui: &mut egui::Ui, label: &str, icon: Icon, tint: Color32, count: usize, active: bool, target: Filter, draggable: bool) -> egui::Response {
        let pal = self.pal;
        let sense = if draggable { Sense::click_and_drag() } else { Sense::click() };
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), sense);
        let p = ui.painter();
        if active {
            p.rect_filled(rect, CornerRadius::same(6), pal.card_hov);
        } else if resp.hovered() {
            p.rect_filled(rect, CornerRadius::same(6), pal.card);
        }
        let cy = rect.center().y;
        let icon_rect = Rect::from_center_size(Pos2::new(rect.left() + 15.0, cy), Vec2::splat(16.0));
        icon::draw(p, icon, icon_rect, if active { tint } else { tint.gamma_multiply(0.9) });
        p.text(Pos2::new(rect.left() + 30.0, cy), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(13.0), if active { pal.txt } else { pal.txt_dim });
        p.text(Pos2::new(rect.right() - 8.0, cy), egui::Align2::RIGHT_CENTER, count.to_string(), egui::FontId::proportional(11.0), pal.txt_dim);
        if resp.clicked() {
            self.filter = target;
        }
        resp
    }

    /// A collapsible sidebar section header (chevron + label + "add"). Returns
    /// `(toggle_clicked, add_clicked)`.
    fn section_header(&self, ui: &mut egui::Ui, label: &str, open: bool) -> (bool, bool) {
        let pal = self.pal;
        let mut toggle = false;
        let mut add = false;
        ui.horizontal(|ui| {
            let chev = if open { Icon::Chevron } else { Icon::ChevronRight };
            if icon::button(ui, chev, 18.0, pal.txt_dim, pal.card).clicked() {
                toggle = true;
            }
            let lbl = ui.add(egui::Label::new(egui::RichText::new(label).color(pal.txt_dim).size(10.0).strong()).sense(Sense::click()));
            if lbl.clicked() {
                toggle = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon::button(ui, Icon::Plus, 18.0, pal.txt_dim, pal.card).clicked() {
                    add = true;
                }
            });
        });
        (toggle, add)
    }

    fn ui_grid(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, pdfs: &[PdfEntry], salt: &str) {
        let card_w = self.card_w;
        let cover_h = card_w / 0.707;
        let spacing = 14.0;
        let avail = ui.available_width();
        let cols = ((avail + spacing) / (card_w + spacing)).floor().max(1.0) as usize;

        let pointer = ui.input(|i| i.pointer.hover_pos());
        let mut rects: Vec<(String, Rect)> = Vec::new();
        egui::Grid::new(("pdf-grid", salt)).spacing(Vec2::new(spacing, spacing)).show(ui, |ui| {
            for (i, pdf) in pdfs.iter().enumerate() {
                let resp = self.ui_card(ui, ctx, pdf, card_w, cover_h);
                rects.push((pdf.id.clone(), resp.rect));
                if (i + 1) % cols == 0 {
                    ui.end_row();
                }
            }
        });

        // Drag-to-reorder within this group: when the dragged card hovers over
        // another card here, move it to that card's slot in the library order.
        if let Some(drag_id) = self.drag_pdf.clone() {
            let in_group = pdfs.iter().any(|p| p.id == drag_id);
            if in_group {
                if let Some(ptr) = pointer {
                    if let Some((target, _)) = rects.iter().find(|(id, r)| id != &drag_id && r.contains(ptr)) {
                        let from = self.data.pdfs.iter().position(|p| p.id == drag_id);
                        let to = self.data.pdfs.iter().position(|p| &p.id == target);
                        if let (Some(from), Some(to)) = (from, to) {
                            // `to` is the target's index in the original order; after
                            // removing `from` it lands just past a forward-moved item
                            // and just before a backward-moved one — an intuitive swap.
                            let item = self.data.pdfs.remove(from);
                            self.data.pdfs.insert(to.min(self.data.pdfs.len()), item);
                        }
                    }
                }
            }
        }
    }

    /// "All PDFs", grouped into topic sections (a PDF may appear under several).
    fn ui_grouped(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, pdfs: &[PdfEntry]) {
        let topics = self.data.topics.clone();
        for t in &topics {
            let group: Vec<PdfEntry> = pdfs.iter().filter(|p| p.in_topic(&t.id)).cloned().collect();
            if group.is_empty() {
                continue;
            }
            self.ui_group_header(ui, Icon::Folder, &t.name, parse_hex(&t.color), group.len());
            self.ui_grid(ui, ctx, &group, &t.id);
            ui.add_space(16.0);
        }
        let unsorted: Vec<PdfEntry> = pdfs.iter().filter(|p| p.topic_ids.is_empty()).cloned().collect();
        if !unsorted.is_empty() {
            self.ui_group_header(ui, Icon::Inbox, "Unsorted", self.pal.txt_dim, unsorted.len());
            self.ui_grid(ui, ctx, &unsorted, "unsorted");
        }
    }

    fn ui_group_header(&self, ui: &mut egui::Ui, ic: Icon, name: &str, accent: Color32, count: usize) {
        let pal = self.pal;
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
            icon::draw(ui.painter(), ic, r, accent);
            ui.label(egui::RichText::new(name).color(pal.txt).size(14.0).strong());
            ui.label(egui::RichText::new(count.to_string()).color(pal.txt_dim).size(12.0));
        });
        let sep_y = ui.cursor().top() + 2.0;
        ui.painter().hline(ui.max_rect().x_range(), sep_y, Stroke::new(1.0_f32, pal.border));
        ui.add_space(8.0);
    }

    fn ui_card(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context, pdf: &PdfEntry, w: f32, cover_h: f32) -> egui::Response {
        let pal = self.pal;
        let total_h = cover_h + 46.0;
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, total_h), Sense::click_and_drag());
        let dragging = self.drag_pdf.as_deref() == Some(pdf.id.as_str());
        if resp.drag_started() {
            self.drag_pdf = Some(pdf.id.clone());
        }
        let popup_id = egui::Id::new(("card-menu", pdf.id.as_str()));
        let popup_open = ui.memory(|m| m.is_popup_open(popup_id));
        let p = ui.painter().clone();
        let active = resp.hovered() || popup_open || dragging;
        p.rect_filled(rect, CornerRadius::same(10), if active { pal.card_hov } else { pal.card });
        let border = if dragging { Stroke::new(1.6_f32, pal.accent) } else { Stroke::new(1.0_f32, if active { pal.accent.gamma_multiply(0.6) } else { pal.border }) };
        p.rect_stroke(rect, CornerRadius::same(10), border, egui::StrokeKind::Inside);

        let cover_rect = Rect::from_min_size(rect.min, Vec2::new(w, cover_h));

        // ensure cover
        self.ensure_cover(pdf);
        match self.covers.get(&pdf.id) {
            Some(Cover::Ready(tex)) => {
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                p.image(tex.id(), cover_rect.shrink(1.0), uv, Color32::WHITE);
            }
            Some(Cover::Failed) => {
                p.text(cover_rect.center(), egui::Align2::CENTER_CENTER, "PDF", egui::FontId::proportional(20.0), pal.txt_dim);
            }
            _ => {
                p.text(cover_rect.center(), egui::Align2::CENTER_CENTER, "…", egui::FontId::proportional(20.0), pal.txt_dim);
            }
        }

        // topic chips overlaid on the cover, tinted with the topic colour
        let topic_chips: Vec<(String, Color32)> = pdf
            .topic_ids
            .iter()
            .filter_map(|id| self.data.topic(id))
            .map(|t| (t.name.clone(), parse_hex(&t.color)))
            .collect();
        if let Some((name, col)) = topic_chips.first() {
            let label = elide(name, 16);
            let font = egui::FontId::proportional(10.0);
            let galley = p.layout_no_wrap(label.clone(), font.clone(), Color32::WHITE);
            let extra = if topic_chips.len() > 1 { format!("  +{}", topic_chips.len() - 1) } else { String::new() };
            let cw = (galley.size().x + 14.0 + extra.len() as f32 * 6.0).min(w - 12.0);
            let chip = Rect::from_min_size(Pos2::new(cover_rect.left() + 6.0, cover_rect.bottom() - 22.0), Vec2::new(cw, 16.0));
            p.rect_filled(chip, CornerRadius::same(8), col.gamma_multiply(0.9));
            p.text(chip.left_center() + Vec2::new(7.0, 0.0), egui::Align2::LEFT_CENTER, format!("{label}{extra}"), font, Color32::from_rgb(0x1a, 0x18, 0x14));
        }

        // name + meta
        let name_pos = Pos2::new(rect.left() + 9.0, cover_rect.bottom() + 8.0);
        p.text(name_pos, egui::Align2::LEFT_TOP, elide(&pdf.name, 22), egui::FontId::proportional(12.0), pal.txt);
        let meta = match topic_chips.len() {
            0 => fmt_size(pdf.size),
            1 => format!("{} · {}", fmt_size(pdf.size), topic_chips[0].0),
            n => format!("{} · {n} topics", fmt_size(pdf.size)),
        };
        p.text(Pos2::new(rect.left() + 9.0, cover_rect.bottom() + 26.0), egui::Align2::LEFT_TOP, elide(&meta, 26), egui::FontId::proportional(10.5), pal.txt_dim);

        if !pdf.exists {
            p.text(Pos2::new(rect.left() + 6.0, rect.top() + 6.0), egui::Align2::LEFT_TOP, "⚠ missing", egui::FontId::proportional(10.0), Color32::from_rgb(0xd9, 0x65, 0x65));
        }

        // hover overflow (three-dots) button → same menu as right-click
        let dots_rect = Rect::from_min_size(Pos2::new(cover_rect.right() - 26.0, cover_rect.top() + 6.0), Vec2::splat(20.0));
        let dots_resp = ui.interact(dots_rect, popup_id.with("dots"), Sense::click());
        if active {
            p.rect_filled(dots_rect, CornerRadius::same(6), pal.panel.gamma_multiply(0.92));
            p.rect_stroke(dots_rect, CornerRadius::same(6), Stroke::new(1.0_f32, pal.border), egui::StrokeKind::Inside);
            icon::draw(&p, Icon::DotsV, dots_rect, pal.txt);
        }
        if dots_resp.clicked() {
            ui.memory_mut(|m| m.toggle_popup(popup_id));
        }
        egui::popup_below_widget(ui, popup_id, &dots_resp, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
            ui.set_min_width(190.0);
            self.card_menu(ui, pdf);
        });

        // open the reader on a plain card click (a drag reorders instead)
        if resp.clicked() && !dots_resp.hovered() && !popup_open {
            self.open_reader(pdf);
        }
        if dragging {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        resp.context_menu(|ui| self.card_menu(ui, pdf));
        resp
    }

    fn card_menu(&mut self, ui: &mut egui::Ui, pdf: &PdfEntry) {
        let pal = self.pal;
        if ui.button("Open").clicked() {
            self.open_reader(pdf);
            ui.close_menu();
        }
        ui.separator();
        ui.label(egui::RichText::new("Topics").color(pal.txt_dim).size(10.0));
        let topics = self.data.topics.clone();
        // Multi-select: a PDF can belong to several topics at once.
        for t in &topics {
            let has = pdf.in_topic(&t.id);
            if ui.selectable_label(has, &t.name).clicked() {
                if let Some(p) = self.data.pdfs.iter_mut().find(|p| p.id == pdf.id) {
                    if has { p.topic_ids.retain(|x| x != &t.id); } else { p.topic_ids.push(t.id.clone()); }
                }
                self.save();
            }
        }
        if ui.button(egui::RichText::new("+ New topic…").color(pal.accent)).clicked() {
            self.show_new_topic = true;
            self.edit_name.clear();
            self.edit_color = PALETTE[0].to_string();
            self.pending_topic_pdf = Some(pdf.id.clone());
            ui.close_menu();
        }
        ui.separator();
        ui.label(egui::RichText::new("Tags").color(pal.txt_dim).size(10.0));
        let tags = self.data.tags.clone();
        for t in &tags {
            let has = pdf.tag_ids.contains(&t.id);
            if ui.selectable_label(has, &t.name).clicked() {
                if let Some(p) = self.data.pdfs.iter_mut().find(|p| p.id == pdf.id) {
                    if has { p.tag_ids.retain(|x| x != &t.id); } else { p.tag_ids.push(t.id.clone()); }
                }
                self.save();
            }
        }
        ui.separator();
        if ui.button("Remove from library").clicked() {
            let path = pdf.path.clone();
            self.data.pdfs.retain(|p| p.id != pdf.id);
            self.data.highlights.remove(&pdf.id);
            self.covers.remove(&pdf.id);
            engine::forget_cover(&path);
            self.save();
            ui.close_menu();
        }
    }

    fn ensure_cover(&mut self, pdf: &PdfEntry) {
        if self.covers.contains_key(&pdf.id) {
            return;
        }
        if !pdf.exists || self.engine_err.is_some() {
            self.covers.insert(pdf.id.clone(), Cover::Failed);
            return;
        }
        self.covers.insert(pdf.id.clone(), Cover::Loading);
        self.worker.send(Req::Cover {
            id: pdf.id.clone(),
            path: pdf.path.clone(),
            target_w: COVER_W,
        });
    }

    // ── Modals ──────────────────────────────────────────────────────────────
    fn ui_modals(&mut self, ctx: &egui::Context) {
        let pal = self.pal;

        // Confirm before deleting a topic.
        if let Some(id) = self.confirm_delete_topic.clone() {
            let name = self.data.topic(&id).map(|t| t.name.clone()).unwrap_or_default();
            let mut open = true;
            let (mut confirm, mut cancel) = (false, false);
            egui::Window::new("Delete topic")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(egui::RichText::new(format!("Delete the topic “{name}”?")).color(pal.txt).size(13.0));
                    ui.label(egui::RichText::new("The PDFs stay in your library; only the topic is removed.").color(pal.txt_dim).size(11.0));
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::new(egui::RichText::new("Delete").color(Color32::WHITE)).fill(Color32::from_rgb(0xc0, 0x4a, 0x4a))).clicked() {
                            confirm = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            if confirm {
                self.data.pdfs.iter_mut().for_each(|p| p.topic_ids.retain(|x| x != &id));
                self.data.topics.retain(|x| x.id != id);
                if matches!(&self.filter, Filter::Topic(f) if f == &id) {
                    self.filter = Filter::All;
                }
                self.drag_topic = None;
                self.save();
                self.confirm_delete_topic = None;
            } else if cancel || !open {
                self.confirm_delete_topic = None;
            }
        }

        if self.show_new_topic {
            let mut open = true;
            let mut create = false;
            egui::Window::new("New Topic").collapsible(false).resizable(false).open(&mut open).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.edit_name).hint_text(egui::RichText::new("Topic name").color(pal.txt_dim.gamma_multiply(0.55))));
                ui.add_space(6.0);
                color_picker(ui, &pal, &mut self.edit_color);
                ui.add_space(10.0);
                if ui.add(egui::Button::new(egui::RichText::new("Create").color(Color32::BLACK)).fill(pal.accent)).clicked() {
                    create = true;
                }
            });
            if create && !self.edit_name.trim().is_empty() {
                let id = uuid::Uuid::new_v4().to_string();
                self.data.topics.push(Topic { id: id.clone(), name: self.edit_name.trim().to_string(), color: self.edit_color.clone() });
                // If launched from a PDF's menu, assign the new topic to it.
                if let Some(pdf_id) = self.pending_topic_pdf.take() {
                    if let Some(p) = self.data.pdfs.iter_mut().find(|p| p.id == pdf_id) {
                        if !p.topic_ids.contains(&id) { p.topic_ids.push(id); }
                    }
                }
                self.save();
                self.show_new_topic = false;
            }
            if !open { self.show_new_topic = false; self.pending_topic_pdf = None; }
        }

        if self.show_new_tag {
            let mut open = true;
            let mut create = false;
            egui::Window::new("New Tag").collapsible(false).resizable(false).open(&mut open).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.edit_name).hint_text(egui::RichText::new("Tag name").color(pal.txt_dim.gamma_multiply(0.55))));
                ui.add_space(6.0);
                color_picker(ui, &pal, &mut self.edit_color);
                ui.add_space(10.0);
                if ui.add(egui::Button::new(egui::RichText::new("Create").color(Color32::BLACK)).fill(pal.accent)).clicked() {
                    create = true;
                }
            });
            if create && !self.edit_name.trim().is_empty() {
                self.data.tags.push(Tag { id: uuid::Uuid::new_v4().to_string(), name: self.edit_name.trim().to_string(), color: self.edit_color.clone() });
                self.save();
                self.show_new_tag = false;
            }
            if !open { self.show_new_tag = false; }
        }
    }

    // ── Reader ──────────────────────────────────────────────────────────────
    fn ui_reader(&mut self, ctx: &egui::Context) {
        let pal = self.pal;
        // toolbar
        let mut zoom_delta = 0.0f32;
        let mut do_copy = false;
        let mut toggle_outline = false;
        egui::TopBottomPanel::top("reader-tb")
            .exact_height(40.0)
            .frame(egui::Frame::new().fill(pal.panel).inner_margin(egui::Margin::symmetric(10, 0)))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    if icon::button(ui, Icon::Contents, 26.0, pal.txt, pal.card_hov).on_hover_text("Show / hide contents").clicked() {
                        toggle_outline = true;
                    }
                    ui.separator();
                    if let Some(r) = &mut self.reader {
                        let total = if r.page_count == 0 { "…".to_string() } else { r.page_count.to_string() };
                        ui.label(egui::RichText::new(format!("Page {} / {}", r.current_page, total)).color(pal.txt_dim).size(12.0));
                        ui.separator();
                        if ui.button(" −  ").clicked() { zoom_delta = -0.2; }
                        ui.label(egui::RichText::new(format!("{}%", (r.zoom * 100.0).round() as i32)).color(pal.txt_dim).size(12.0));
                        if ui.button(" +  ").clicked() { zoom_delta = 0.2; }
                        ui.separator();
                        // ISOTYPE tool selector: one pictogram per pointer mode.
                        let active_bg = pal.accent.gamma_multiply(0.32);
                        for (t, ic, tip) in [
                            (ReaderTool::Pan, Icon::Hand, "Pan, drag to move the page (1)"),
                            (ReaderTool::Select, Icon::Cursor, "Select text (2)"),
                            (ReaderTool::Highlight, Icon::Marker, "Select & highlight (3)"),
                        ] {
                            if icon::toggle(ui, ic, 26.0, pal.txt, active_bg, pal.card_hov, r.tool == t)
                                .on_hover_text(tip)
                                .clicked()
                            {
                                r.tool = t;
                            }
                        }
                        // The highlight palette appears only once the marker is
                        // chosen, rather than always occupying the toolbar.
                        if r.tool == ReaderTool::Highlight {
                            ui.separator();
                            for c in HL_COLORS {
                                let sel = &r.hl_color == c;
                                let (rect, resp) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
                                ui.painter().circle_filled(rect.center(), 8.0, parse_hex(c));
                                if sel {
                                    ui.painter().circle_stroke(rect.center(), 9.0, Stroke::new(2.0_f32, pal.txt));
                                }
                                if resp.clicked() { r.hl_color = c.to_string(); }
                            }
                        }
                        if r.selection.as_ref().map(|s| !s.text.is_empty()).unwrap_or(false) {
                            ui.separator();
                            if ui.button("Copy").clicked() { do_copy = true; }
                        }
                    }
                });
            });

        if toggle_outline {
            self.show_outline = !self.show_outline;
        }
        if zoom_delta != 0.0 {
            if let Some(r) = &mut self.reader {
                r.zoom = (r.zoom + zoom_delta).clamp(0.5, 4.0);
            }
        }
        if do_copy {
            if let Some(r) = &self.reader {
                if let Some(sel) = &r.selection {
                    ctx.copy_text(sel.text.clone());
                }
            }
            self.toast(ctx, "Copied");
        }

        // keyboard: Ctrl+C copy, Ctrl +/- zoom
        let (copy_key, zin, zout) = ctx.input(|i| (
            i.modifiers.command && i.key_pressed(egui::Key::C),
            i.modifiers.command && i.key_pressed(egui::Key::Plus),
            i.modifiers.command && i.key_pressed(egui::Key::Minus),
        ));
        if copy_key {
            if let Some(r) = &self.reader {
                if let Some(sel) = &r.selection { ctx.copy_text(sel.text.clone()); }
            }
        }
        if let Some(r) = &mut self.reader {
            if zin { r.zoom = (r.zoom + 0.2).clamp(0.5, 4.0); }
            if zout { r.zoom = (r.zoom - 0.2).clamp(0.5, 4.0); }
        }

        // Number keys switch tools — but not while a text field (the page jump
        // box) is focused, where they are ordinary input.
        if ctx.memory(|m| m.focused()).is_none() {
            let (t1, t2, t3) = ctx.input(|i| (
                i.key_pressed(egui::Key::Num1),
                i.key_pressed(egui::Key::Num2),
                i.key_pressed(egui::Key::Num3),
            ));
            if let Some(r) = &mut self.reader {
                if t1 { r.tool = ReaderTool::Pan; }
                if t2 { r.tool = ReaderTool::Select; }
                if t3 { r.tool = ReaderTool::Highlight; }
            }
        }

        // Okular-style contents panel: jump-to-page + table of contents.
        if self.show_outline {
            egui::SidePanel::left("outline")
                .default_width(240.0)
                .width_range(170.0..=440.0)
                .resizable(true)
                .frame(egui::Frame::new().fill(pal.panel).inner_margin(egui::Margin::same(10)))
                .show(ctx, |ui| self.ui_outline(ui));
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(pal.page_bg))
            .show(ctx, |ui| {
                self.ui_pages(ui, ctx);
            });
    }

    fn ui_outline(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal;
        let Some(r) = self.reader.as_mut() else { return };

        // Jump-to-page box.
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Page").color(pal.txt_dim).size(12.0));
            let resp = ui.add(egui::TextEdit::singleline(&mut r.jump_input).desired_width(48.0).hint_text(egui::RichText::new("#").color(pal.txt_dim.gamma_multiply(0.55))));
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let go = ui.small_button("Go").clicked();
            if (enter || go) && r.page_count > 0 {
                if let Ok(n) = r.jump_input.trim().parse::<u32>() {
                    r.scroll_to_page = Some(n.clamp(1, r.page_count));
                }
            }
            ui.label(egui::RichText::new(format!("/ {}", if r.page_count == 0 { "…".into() } else { r.page_count.to_string() })).color(pal.txt_dim).size(11.0));
        });
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(4.0);

        if r.outline.is_empty() {
            ui.label(egui::RichText::new("No table of contents").color(pal.txt_dim).size(11.0));
            return;
        }
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
            icon::draw(ui.painter(), Icon::Contents, r, pal.txt_dim);
            ui.label(egui::RichText::new("CONTENTS").color(pal.txt_dim).size(10.0).strong());
        });
        ui.add_space(4.0);

        // Which entry are we currently reading? The deepest item whose page is
        // at or before the current page.
        let cur_page = r.current_page;
        let active = r
            .outline
            .iter()
            .enumerate()
            .filter(|(_, it)| it.page.map(|p| p + 1 <= cur_page).unwrap_or(false))
            .map(|(i, _)| i)
            .last();

        let mut jump: Option<u32> = None;
        let mut toggle: Option<usize> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let avail = ui.available_width();
            // While collapsed, hide every deeper item until the level returns.
            let mut hide_below: Option<u32> = None;
            for (idx, it) in r.outline.iter().enumerate() {
                if let Some(l) = hide_below {
                    if it.level > l {
                        continue;
                    }
                    hide_below = None;
                }
                // A subtle, pale separator before each top-level chapter.
                if it.level == 0 && idx != 0 {
                    ui.add_space(3.0);
                    let (sr, _) = ui.allocate_exact_size(Vec2::new(avail, 1.0), Sense::hover());
                    ui.painter().hline(sr.x_range(), sr.center().y, Stroke::new(1.0_f32, pal.border.gamma_multiply(0.45)));
                    ui.add_space(3.0);
                }
                let has_children = r.outline.get(idx + 1).map(|n| n.level > it.level).unwrap_or(false);
                let collapsed = r.collapsed.contains(&idx);
                let is_active = active == Some(idx);
                let clickable = it.page.is_some();
                let sense = if clickable || has_children { Sense::click() } else { Sense::hover() };
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(avail, 22.0), sense);
                let hovered = resp.hovered();
                if is_active {
                    // Highlight the section we're currently reading.
                    ui.painter().rect_filled(rect, CornerRadius::same(5), pal.accent.gamma_multiply(0.16));
                    ui.painter().rect_filled(Rect::from_min_size(rect.left_top(), Vec2::new(2.5, rect.height())), CornerRadius::same(1), pal.accent);
                } else if hovered && (clickable || has_children) {
                    ui.painter().rect_filled(rect, CornerRadius::same(5), pal.card_hov.gamma_multiply(0.7));
                }
                if clickable || has_children {
                    ui.ctx().set_cursor_icon(if hovered { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Default });
                }
                let base = if is_active { pal.accent } else if clickable { pal.txt } else { pal.txt_dim };
                let color = if is_active || hovered { base } else { base.gamma_multiply(0.82) };
                let indent = 8.0 + it.level as f32 * 12.0;
                // Collapse triangle for entries that have sub-entries.
                let mut text_x = rect.left() + indent;
                let chev_rect = Rect::from_center_size(Pos2::new(rect.left() + indent + 6.0, rect.center().y), Vec2::splat(14.0));
                if has_children {
                    let chev = if collapsed { Icon::ChevronRight } else { Icon::Chevron };
                    icon::draw(&ui.painter().with_clip_rect(rect), chev, chev_rect, color.gamma_multiply(0.9));
                    text_x = chev_rect.right() + 3.0;
                }
                ui.painter().with_clip_rect(rect).text(
                    Pos2::new(text_x, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    &it.title,
                    egui::FontId::proportional(12.0),
                    color,
                );
                if resp.clicked() {
                    let on_chev = has_children
                        && resp.interact_pointer_pos().map(|p| chev_rect.expand(4.0).contains(p)).unwrap_or(false);
                    if on_chev {
                        toggle = Some(idx);
                    } else if let Some(p) = it.page {
                        jump = Some(p + 1); // outline pages are 0-based
                    }
                }
                if collapsed {
                    hide_below = Some(it.level);
                }
            }
        });
        if let Some(idx) = toggle {
            if !r.collapsed.remove(&idx) {
                r.collapsed.insert(idx);
            }
        }
        if let Some(p) = jump {
            r.scroll_to_page = Some(p.clamp(1, r.page_count.max(1)));
        }
    }

    /// Ctrl/⌘ + scroll (or trackpad pinch) zoom, anchored under the cursor.
    /// Returns a forced vertical scroll offset for this frame that keeps the
    /// point under the pointer visually fixed, or `None` if nothing zoomed.
    fn apply_scroll_zoom(&mut self, ctx: &egui::Context, viewport_top: f32) -> Option<f32> {
        let (zoom_delta, hover) = ctx.input(|i| (i.zoom_delta(), i.pointer.hover_pos()));
        if (zoom_delta - 1.0).abs() < 1e-3 {
            return None;
        }
        let r = self.reader.as_mut()?;
        let old = r.zoom;
        let new = (old * zoom_delta).clamp(0.5, 5.0);
        if (new - old).abs() < 1e-4 {
            return None;
        }
        r.zoom = new;
        // Keep the content point under the cursor fixed: scale the distance from
        // the viewport top through the zoom factor.
        let f = new / old;
        let anchor = (hover?.y - viewport_top).max(0.0);
        Some(((r.scroll_offset + anchor) * f - anchor).max(0.0))
    }

    fn ui_pages(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let pal = self.pal;
        // Ctrl/⌘ + scroll (or trackpad pinch) zooms, anchored under the cursor —
        // must run before we snapshot `zoom`, and it may force a scroll offset.
        let viewport_top = ui.max_rect().top();
        let mut forced_offset = self.apply_scroll_zoom(ctx, viewport_top);

        let Some(reader) = self.reader.as_ref() else { return };

        if let Some(err) = &reader.open_failed {
            ui.centered_and_justified(|ui| {
                ui.colored_label(Color32::from_rgb(0xd9, 0x65, 0x65), format!("Could not open document:\n{err}"));
            });
            return;
        }
        if reader.page_count == 0 {
            ui.centered_and_justified(|ui| {
                ui.label(egui::RichText::new("Opening…").color(pal.txt_dim).size(14.0));
            });
            ctx.request_repaint();
            return;
        }

        let zoom = reader.zoom;
        let tool = reader.tool;
        let path = reader.path.clone();
        let pdf_id = reader.pdf_id.clone();
        let hl_color = reader.hl_color.clone();
        let default_size = reader.default_size;
        let page_count = reader.page_count;
        let highlights = self.data.highlights.get(&pdf_id).cloned().unwrap_or_default();

        let reader = self.reader.as_mut().unwrap();

        let mut new_selection: Option<Selection> = None;
        let mut save_highlight: Option<PdfHighlight> = None;
        let mut remove_highlight: Option<String> = None;
        let mut start_confirm: Option<(String, Pos2)> = None; // clicked a highlight → ask
        let mut deselect = false; // a plain click on empty text clears the selection
        let mut requests: Vec<Req> = Vec::new();
        let mut top_visible_page = reader.current_page;
        let mut first_visible_set = false;

        // A pending page jump (from the contents panel) wins over zoom anchoring:
        // sum the heights of the pages above the target to get its scroll offset.
        if let Some(target) = reader.scroll_to_page.take() {
            let mut y = 12.0f32; // matches the top padding added before the first page
            let n = (target.saturating_sub(1) as usize).min(reader.pages.len());
            for slot in &reader.pages[..n] {
                let (_, h) = slot.size_pts.unwrap_or(default_size);
                y += h * zoom + 10.0; // page height + inter-page spacing
            }
            forced_offset = Some(y);
            reader.current_page = target;
        }

        let mut area = egui::ScrollArea::both()
            .auto_shrink([false, false])
            // In Pan mode a drag moves the page; otherwise drags select text.
            .drag_to_scroll(tool == ReaderTool::Pan);
        if let Some(off) = forced_offset {
            area = area.vertical_scroll_offset(off);
        }
        let area_out = area
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(12.0);
                    for idx in 0..page_count {
                        let slot = &mut reader.pages[idx as usize];
                        let (w_pts, h_pts) = slot.size_pts.unwrap_or(default_size);
                        let disp = Vec2::new(w_pts * zoom, h_pts * zoom);
                        // Build the line index once glyphs are available.
                        if slot.lines.is_none() && slot.chars.is_some() {
                            slot.lines = Some(build_lines(slot.chars.as_ref().unwrap()));
                        }

                        // In Pan mode the page only senses hover, so drags fall
                        // through to the scroll area and move the page.
                        let sense = if tool == ReaderTool::Pan { Sense::hover() } else { Sense::click_and_drag() };
                        let (rect, resp) = ui.allocate_exact_size(disp, sense);
                        let visible = ui.is_rect_visible(rect);
                        if visible && !first_visible_set {
                            top_visible_page = idx + 1;
                            first_visible_set = true;
                        }

                        if visible {
                            // (re)render texture at the current zoom if needed
                            if (slot.texture.is_none() || (slot.rendered_zoom - zoom).abs() > 0.001)
                                && (slot.requested_zoom - zoom).abs() > 0.001
                            {
                                requests.push(Req::Render { path: path.clone(), page: idx, scale: zoom });
                                slot.requested_zoom = zoom;
                            }
                            if slot.chars.is_none() && !slot.chars_requested {
                                requests.push(Req::Chars { path: path.clone(), page: idx });
                                slot.chars_requested = true;
                            }

                            let p = ui.painter_at(rect);
                            p.rect_filled(rect, CornerRadius::same(2), Color32::WHITE);
                            if let Some(tex) = &slot.texture {
                                p.image(tex.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                            } else {
                                p.text(rect.center(), egui::Align2::CENTER_CENTER, "…", egui::FontId::proportional(18.0), pal.txt_dim);
                            }

                            // draw stored highlights for this page
                            for hl in highlights.iter().filter(|h| h.page == idx + 1) {
                                let col = parse_hex(&hl.color).gamma_multiply(0.42);
                                for r in &hl.rects {
                                    let hr = Rect::from_min_size(
                                        Pos2::new(rect.left() + r.x * disp.x, rect.top() + r.y * disp.y),
                                        Vec2::new(r.w * disp.x, r.h * disp.y),
                                    );
                                    p.rect_filled(hr, CornerRadius::same(2), col);
                                }
                            }

                            // selection overlay (only for the page holding the selection).
                            // Translucent blue ribbon, à la Okular: one uniform-height row
                            // per text line, from the first to the last selected glyph.
                            if let Some(sel) = &reader.selection {
                                if sel.page == idx + 1 {
                                    if let (Some(chars), Some(lines)) = (&slot.chars, &slot.lines) {
                                        for (x0, y0, x1, y1) in selection_rects(chars, lines, sel.start, sel.end) {
                                            let sr = Rect::from_min_max(
                                                Pos2::new(rect.left() + x0 * zoom, rect.top() + y0 * zoom),
                                                Pos2::new(rect.left() + x1 * zoom, rect.top() + y1 * zoom),
                                            );
                                            p.rect_filled(sr, CornerRadius::same(2), pal.select_fill);
                                        }
                                    }
                                }
                            }

                            // Pan mode: no text interaction, just a grab cursor;
                            // the drag is consumed by the scroll area.
                            if tool == ReaderTool::Pan {
                                if resp.hovered() {
                                    let down = ui.input(|i| i.pointer.primary_down());
                                    ui.ctx().set_cursor_icon(if down { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
                                }
                            }
                            // text interaction: I-beam cursor, drag-select, word-select, hit-testing
                            else if let (Some(chars), Some(lines)) = (&slot.chars, &slot.lines) {
                                // Which stored highlight (if any) is under a page-point position?
                                let over_highlight = |pos: Pos2| -> Option<String> {
                                    let fx = (pos.x - rect.left()) / disp.x;
                                    let fy = (pos.y - rect.top()) / disp.y;
                                    highlights
                                        .iter()
                                        .filter(|h| h.page == idx + 1)
                                        .find(|h| h.rects.iter().any(|r| fx >= r.x && fx <= r.x + r.w && fy >= r.y && fy <= r.y + r.h))
                                        .map(|h| h.id.clone())
                                };
                                // Pointer position → caret index (0..=len) in page points.
                                let caret = |pos: Pos2| caret_at(chars, lines, (pos.x - rect.left()) / zoom, (pos.y - rect.top()) / zoom);

                                // Desktop-style cursor: I-beam over text, hand over a highlight.
                                if resp.dragged() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
                                } else if resp.hovered() {
                                    let on_hl = resp.hover_pos().and_then(&over_highlight).is_some();
                                    ui.ctx().set_cursor_icon(if on_hl { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Text });
                                }

                                if resp.triple_clicked() {
                                    // Triple-click selects the whole line under the pointer.
                                    if let Some(pos) = resp.interact_pointer_pos() {
                                        let px = (pos.x - rect.left()) / zoom;
                                        let py = (pos.y - rect.top()) / zoom;
                                        if let Some(ci) = char_at(chars, lines, px, py) {
                                            let (lo, hi) = line_range(lines, ci);
                                            let text = selection_text(chars, lo, hi);
                                            new_selection = Some(Selection { page: idx + 1, start: lo, end: hi, text });
                                        }
                                    }
                                } else if resp.double_clicked() {
                                    // Double-click selects the whole word under the pointer.
                                    if let Some(pos) = resp.interact_pointer_pos() {
                                        let px = (pos.x - rect.left()) / zoom;
                                        let py = (pos.y - rect.top()) / zoom;
                                        if let Some(ci) = char_at(chars, lines, px, py) {
                                            let (lo, hi) = word_range(chars, ci);
                                            let text = selection_text(chars, lo, hi);
                                            new_selection = Some(Selection { page: idx + 1, start: lo, end: hi, text });
                                        }
                                    }
                                } else if resp.drag_started() {
                                    if let Some(pos) = resp.interact_pointer_pos() {
                                        let c = caret(pos);
                                        new_selection = Some(Selection { page: idx + 1, start: c, end: c, text: String::new() });
                                    }
                                } else if resp.dragged() {
                                    if let (Some(pos), Some(cur)) = (resp.interact_pointer_pos(), &reader.selection) {
                                        if cur.page == idx + 1 {
                                            new_selection = Some(Selection { page: idx + 1, start: cur.start, end: caret(pos), text: String::new() });
                                        }
                                    }
                                } else if resp.clicked() {
                                    // Click a highlight → ask for confirmation near the
                                    // cursor; click empty text → clear the selection.
                                    if let Some(pos) = resp.interact_pointer_pos() {
                                        match over_highlight(pos) {
                                            Some(id) => start_confirm = Some((id, pos)),
                                            None => deselect = true,
                                        }
                                    }
                                }

                                // finished a drag → materialise selection text, and
                                // in Highlight mode apply the highlight immediately.
                                if resp.drag_stopped() {
                                    if let Some(cur) = &reader.selection {
                                        if cur.page == idx + 1 {
                                            let text = selection_text(chars, cur.start, cur.end);
                                            if tool == ReaderTool::Highlight && !text.is_empty() {
                                                let rects = selection_rects(chars, lines, cur.start, cur.end)
                                                    .into_iter()
                                                    .map(|(x0, y0, x1, y1)| HighlightRect {
                                                        x: x0 / w_pts, y: y0 / h_pts, w: (x1 - x0) / w_pts, h: (y1 - y0) / h_pts,
                                                    })
                                                    .collect();
                                                save_highlight = Some(PdfHighlight {
                                                    id: uuid::Uuid::new_v4().to_string(),
                                                    page: cur.page,
                                                    rects,
                                                    color: hl_color.clone(),
                                                });
                                            }
                                            new_selection = Some(Selection { page: cur.page, start: cur.start, end: cur.end, text });
                                        }
                                    }
                                }

                                // Right-click on a live selection to highlight or copy it.
                                let sel_here = reader
                                    .selection
                                    .as_ref()
                                    .filter(|s| s.page == idx + 1 && s.start != s.end)
                                    .is_some();
                                resp.context_menu(|ui| {
                                    if let Some(sel) = reader.selection.as_ref().filter(|_| sel_here) {
                                        ui.label(egui::RichText::new("Highlight").color(pal.txt_dim).size(10.0).strong());
                                        ui.horizontal(|ui| {
                                            for c in HL_COLORS {
                                                let (r, cr) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
                                                ui.painter().circle_filled(r.center(), 8.0, parse_hex(c));
                                                if cr.clicked() {
                                                    let rects = selection_rects(chars, lines, sel.start, sel.end)
                                                        .into_iter()
                                                        .map(|(x0, y0, x1, y1)| HighlightRect {
                                                            x: x0 / w_pts, y: y0 / h_pts, w: (x1 - x0) / w_pts, h: (y1 - y0) / h_pts,
                                                        })
                                                        .collect();
                                                    save_highlight = Some(PdfHighlight {
                                                        id: uuid::Uuid::new_v4().to_string(),
                                                        page: sel.page,
                                                        rects,
                                                        color: c.to_string(),
                                                    });
                                                    ui.close_menu();
                                                }
                                            }
                                        });
                                        ui.separator();
                                        if ui.button("Copy").clicked() {
                                            ui.ctx().copy_text(selection_text(chars, sel.start, sel.end));
                                            ui.close_menu();
                                        }
                                    } else {
                                        ui.label(egui::RichText::new("Select text to highlight").color(pal.txt_dim).size(11.0));
                                    }
                                });
                            }
                        } else {
                            // free textures of off-screen pages on big books to bound memory
                            if slot.texture.is_some() && page_count > 40 {
                                slot.texture = None;
                                slot.rendered_zoom = -1.0;
                                slot.requested_zoom = -1.0;
                            }
                            let p = ui.painter_at(rect);
                            p.rect_filled(rect, CornerRadius::same(2), Color32::from_gray(30));
                        }

                        ui.add_space(10.0);
                    }
                });
            });

        reader.scroll_offset = area_out.state.offset.y;

        if let Some(sel) = new_selection {
            reader.selection = Some(sel);
            reader.pending_delete = None; // starting a selection dismisses the prompt
        } else if deselect {
            reader.selection = None;
            reader.pending_delete = None;
        }
        if let Some((id, pos)) = start_confirm {
            reader.pending_delete = Some((id, pos));
        }
        reader.current_page = top_visible_page;

        // Delete-confirmation popup, anchored just off the cursor where the
        // highlight was clicked. Nothing is removed until "Delete" is pressed.
        if let Some((id, pos)) = reader.pending_delete.clone() {
            let mut confirm = false;
            let mut cancel = false;
            egui::Area::new(egui::Id::new("hl-del"))
                .order(egui::Order::Foreground)
                .fixed_pos(pos + Vec2::new(8.0, 8.0))
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(pal.card)
                        .stroke(Stroke::new(1.0_f32, pal.border))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(egui::Margin::same(10))
                        .show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new("Delete this highlight?").color(pal.txt).size(13.0));
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    if ui.add(egui::Button::new(egui::RichText::new("Delete").color(Color32::WHITE)).fill(Color32::from_rgb(0xc0, 0x4a, 0x4a))).clicked() {
                                        confirm = true;
                                    }
                                    if ui.button("Cancel").clicked() {
                                        cancel = true;
                                    }
                                });
                            });
                        });
                });
            if confirm {
                remove_highlight = Some(id);
                reader.pending_delete = None;
            } else if cancel {
                reader.pending_delete = None;
            }
        }

        for req in requests {
            self.worker.send(req);
        }

        if let Some(hl) = save_highlight {
            self.data.highlights.entry(pdf_id.clone()).or_default().push(hl);
            self.save();
            if let Some(r) = &mut self.reader { r.selection = None; }
        }
        if let Some(id) = remove_highlight {
            if let Some(v) = self.data.highlights.get_mut(&pdf_id) {
                v.retain(|h| h.id != id);
            }
            self.save();
        }
    }
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn setup_style(ctx: &egui::Context, pal: &Pal, dark: bool) {
    let mut visuals = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    visuals.override_text_color = Some(pal.txt);
    visuals.panel_fill = pal.bg;
    visuals.window_fill = pal.panel;
    visuals.window_stroke = Stroke::new(1.0_f32, pal.border);
    visuals.extreme_bg_color = pal.card;
    visuals.widgets.inactive.bg_fill = pal.card;
    visuals.widgets.hovered.bg_fill = pal.card_hov;
    visuals.widgets.active.bg_fill = pal.card_hov;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, pal.border);
    visuals.selection.bg_fill = pal.accent.gamma_multiply(0.35);
    ctx.set_visuals(visuals);
    // Ctrl +/-/0 should zoom the page only, not egui's whole-UI scale factor.
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
}

fn upload(ctx: &egui::Context, name: &str, img: ColorImage) -> TextureHandle {
    ctx.load_texture(name, img, egui::TextureOptions::LINEAR)
}

fn color_picker(ui: &mut egui::Ui, pal: &Pal, current: &mut String) {
    ui.horizontal_wrapped(|ui| {
        for c in PALETTE {
            let sel = current == c;
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
            ui.painter().circle_filled(rect.center(), 11.0, parse_hex(c));
            if sel {
                ui.painter().circle_stroke(rect.center(), 12.0, Stroke::new(2.0_f32, pal.txt));
            }
            if resp.clicked() {
                *current = c.to_string();
            }
        }
    });
}

/// Group glyphs (in reading order) into text lines. A glyph joins the current
/// line when their vertical extents overlap by more than half the smaller
/// height, *or* when either box's vertical centre lies inside the other's
/// extent. The centre test is essential: pdfium gives space glyphs a
/// zero-height box sitting on the baseline, which has no overlap with the
/// letters' band — without it every space would start a new line, and a
/// selection would render as disconnected words rather than whole sentences.
fn build_lines(chars: &[CharBox]) -> Vec<LineBox> {
    let mut lines: Vec<LineBox> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        let (mut top, mut bottom) = (chars[i].y, chars[i].y + chars[i].h);
        let (mut left, mut right) = (chars[i].x, chars[i].x + chars[i].w);
        i += 1;
        while i < chars.len() {
            let c = &chars[i];
            let (c_top, c_bottom) = (c.y, c.y + c.h);
            let c_center = (c_top + c_bottom) * 0.5;
            let line_center = (top + bottom) * 0.5;
            let overlap = bottom.min(c_bottom) - top.max(c_top);
            let min_h = (bottom - top).min(c.h).max(1.0);
            let joins = overlap > 0.5 * min_h
                || (c_center >= top && c_center <= bottom)
                || (line_center >= c_top && line_center <= c_bottom);
            if joins {
                top = top.min(c_top);
                bottom = bottom.max(c_bottom);
                left = left.min(c.x);
                right = right.max(c.x + c.w);
                i += 1;
            } else {
                break;
            }
        }
        lines.push(LineBox { start, end: i, top, bottom, left, right });
    }
    lines
}

/// Distance from `p` to the closed interval `[lo, hi]` (0 when inside).
fn axis_dist(lo: f32, hi: f32, p: f32) -> f32 {
    if p < lo {
        lo - p
    } else if p > hi {
        p - hi
    } else {
        0.0
    }
}

/// The line best matching a page-point position: vertical distance dominates
/// (so we land on the right row), horizontal distance breaks ties — which lets
/// multiple columns or same-height lines resolve by which one the pointer is in.
fn line_at(lines: &[LineBox], px: f32, py: f32) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut best_key = (f32::MAX, f32::MAX);
    for (i, l) in lines.iter().enumerate() {
        let key = (axis_dist(l.top, l.bottom, py), axis_dist(l.left, l.right, px));
        if key.0 < best_key.0 || (key.0 == best_key.0 && key.1 < best_key.1) {
            best_key = key;
            best = Some(i);
        }
    }
    best
}

/// Nearest glyph to a page-point position, restricted to its line (word/line select).
fn char_at(chars: &[CharBox], lines: &[LineBox], px: f32, py: f32) -> Option<usize> {
    let li = line_at(lines, px, py)?;
    let line = lines[li];
    let mut best = line.start;
    let mut best_d = f32::MAX;
    for i in line.start..line.end {
        let c = &chars[i];
        let d = (c.x + c.w * 0.5 - px).abs();
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    Some(best)
}

/// Caret index (0..=len) nearest a page-point position. Snaps to the best line,
/// then to the glyph boundary nearest the pointer within that line — dragging
/// past the last glyph selects to the line's end, like a real editor.
fn caret_at(chars: &[CharBox], lines: &[LineBox], px: f32, py: f32) -> usize {
    let Some(li) = line_at(lines, px, py) else { return 0 };
    let line = lines[li];
    for i in line.start..line.end {
        let c = &chars[i];
        if px < c.x + c.w * 0.5 {
            return i;
        }
    }
    line.end
}

/// Half-open caret range `[lo, hi)` covering the word (a run of non-whitespace
/// glyphs on one line) that contains glyph `i`.
fn word_range(chars: &[CharBox], i: usize) -> (usize, usize) {
    if i >= chars.len() {
        return (chars.len(), chars.len());
    }
    if chars[i].ch.is_whitespace() {
        return (i, i + 1);
    }
    let same_line = |a: &CharBox, b: &CharBox| (a.y - b.y).abs() < a.h.max(b.h) * 0.6;
    let mut lo = i;
    while lo > 0 && !chars[lo - 1].ch.is_whitespace() && same_line(&chars[lo - 1], &chars[lo]) {
        lo -= 1;
    }
    let mut hi = i;
    while hi + 1 < chars.len() && !chars[hi + 1].ch.is_whitespace() && same_line(&chars[hi], &chars[hi + 1]) {
        hi += 1;
    }
    (lo, hi + 1)
}

/// Half-open caret range `[lo, hi)` for the whole line that contains glyph `i`.
fn line_range(lines: &[LineBox], i: usize) -> (usize, usize) {
    for l in lines {
        if i >= l.start && i < l.end {
            return (l.start, l.end);
        }
    }
    (i, i + 1)
}

/// Selection rectangles (page points) for the half-open caret range `[a, b)`:
/// one uniform-height row per intersected text line, spanning from the first to
/// the last selected glyph on that line. Uniform rows avoid the ragged, glyph-
/// hugging boxes that read as "generic" — this is the Okular/Poppler look.
fn selection_rects(chars: &[CharBox], lines: &[LineBox], a: usize, b: usize) -> Vec<(f32, f32, f32, f32)> {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let hi = hi.min(chars.len());
    if lo >= hi {
        return vec![];
    }
    let mut rects = Vec::new();
    for l in lines {
        let s = l.start.max(lo);
        let e = l.end.min(hi);
        if s >= e {
            continue;
        }
        // Walk the line's selected glyphs, merging them into a single row and
        // filling the small gaps between words. A gap wider than a couple of line
        // heights is treated as a column gutter (or table cell break): flush the
        // current run and start a new rect, so two columns never fuse into one bar.
        let gutter = (l.bottom - l.top).max(1.0) * 2.0;
        let mut cur: Option<(f32, f32)> = None; // (x0, x1) of the run in progress
        let mut prev_right = f32::MIN;
        for c in &chars[s..e] {
            let (cl, cr) = (c.x, c.x + c.w);
            cur = match cur {
                Some((x0, x1)) if cl - prev_right <= gutter => Some((x0.min(cl), x1.max(cr))),
                Some((x0, x1)) => {
                    if x1 > x0 {
                        rects.push((x0, l.top, x1, l.bottom));
                    }
                    Some((cl, cr))
                }
                None => Some((cl, cr)),
            };
            prev_right = prev_right.max(cr);
        }
        if let Some((x0, x1)) = cur {
            if x1 > x0 {
                rects.push((x0, l.top, x1, l.bottom));
            }
        }
    }
    rects
}

/// Text for the half-open caret range `[a, b)`, inserting newlines between lines.
fn selection_text(chars: &[CharBox], a: usize, b: usize) -> String {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let hi = hi.min(chars.len());
    if lo >= hi {
        return String::new();
    }
    let mut s = String::new();
    let mut last_y = chars[lo].y;
    for c in &chars[lo..hi] {
        if (c.y - last_y).abs() > c.h * 0.6 {
            s.push('\n');
        }
        s.push(c.ch);
        last_y = c.y;
    }
    s
}

fn merge_into(dst: &mut AppData, src: AppData) {
    for t in src.topics {
        if !dst.topics.iter().any(|x| x.id == t.id) {
            dst.topics.push(t);
        }
    }
    for t in src.tags {
        if !dst.tags.iter().any(|x| x.id == t.id) {
            dst.tags.push(t);
        }
    }
    for p in src.pdfs {
        if !dst.pdfs.iter().any(|x| x.path == p.path) {
            dst.pdfs.push(p);
        }
    }
    for (k, v) in src.highlights {
        dst.highlights.entry(k).or_insert(v);
    }
}

fn fmt_size(b: u64) -> String {
    if b < 1024 {
        format!("{b} B")
    } else if b < 1_048_576 {
        format!("{} KB", b / 1024)
    } else {
        format!("{:.1} MB", b as f64 / 1_048_576.0)
    }
}

/// What a click on a Drive document row means.
#[cfg(feature = "drive")]
enum RowAct {
    None,
    Open,
    Save,
}

/// One document row in the Drive picker: a hoverable, clickable card with a PDF
/// glyph, name and size. Left-click opens; right-click offers open / save.
#[cfg(feature = "drive")]
fn drive_row(ui: &mut egui::Ui, pal: Pal, f: &crate::drive::DriveFile) -> RowAct {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(8), pal.card_hov);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let g = Rect::from_min_size(Pos2::new(rect.left() + 10.0, rect.center().y - 9.0), Vec2::new(14.0, 18.0));
    ui.painter().rect_filled(g, CornerRadius::same(2), pal.card);
    ui.painter().rect_stroke(g, CornerRadius::same(2), Stroke::new(1.0_f32, pal.border), egui::StrokeKind::Middle);
    ui.painter().text(g.center(), egui::Align2::CENTER_CENTER, "P", egui::FontId::proportional(9.0), pal.accent);
    ui.painter().text(
        Pos2::new(g.right() + 12.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        elide(&f.name, 50),
        egui::FontId::proportional(13.0),
        pal.txt,
    );
    if f.size > 0 {
        ui.painter().text(
            Pos2::new(rect.right() - 10.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            fmt_size(f.size),
            egui::FontId::proportional(11.0),
            pal.txt_dim,
        );
    }
    let mut act = if resp.clicked() { RowAct::Open } else { RowAct::None };
    resp.context_menu(|ui| {
        if ui.button("Open").clicked() {
            act = RowAct::Open;
            ui.close_menu();
        }
        if ui.button("Save to device…").clicked() {
            act = RowAct::Save;
            ui.close_menu();
        }
    });
    act
}

fn elide(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    } else {
        s.to_string()
    }
}
