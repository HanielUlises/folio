//! Folio — native egui application.

use crate::engine::{self, Req, Res, Worker};
use crate::model::*;
use crate::pdf::CharBox;
use eframe::egui::{self, Color32, ColorImage, CornerRadius, Pos2, Rect, Sense, Stroke, TextureHandle, Vec2};
use std::collections::HashMap;

// ── Palette (dark, warm accent) ────────────────────────────────────────────
const BG: Color32 = Color32::from_rgb(0x14, 0x14, 0x16);
const PANEL: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x1d);
const CARD: Color32 = Color32::from_rgb(0x20, 0x20, 0x24);
const CARD_HOV: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x30);
const BORDER: Color32 = Color32::from_rgb(0x33, 0x33, 0x3a);
const ACCENT: Color32 = Color32::from_rgb(0xd4, 0xa8, 0x43);
const TXT: Color32 = Color32::from_rgb(0xee, 0xeb, 0xe5);
const TXT_DIM: Color32 = Color32::from_rgb(0x8e, 0x8b, 0x86);
const PAGE_BG: Color32 = Color32::from_rgb(0x21, 0x21, 0x25);
// Live text-selection ribbon (transient), distinct from saved highlight
// annotations. A translucent Breeze-style selection blue (#3daee9) premultiplied
// over the page — reads like a native desktop selection, text stays legible.
const SELECT_FILL: Color32 = Color32::from_rgba_premultiplied(0x19, 0x48, 0x60, 0x69);

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
}

pub struct Folio {
    worker: Worker,
    engine_err: Option<String>,
    data: AppData,
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
    toast: Option<(String, f64)>,
}

impl Folio {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_style(&cc.egui_ctx);
        let worker = Worker::spawn(cc.egui_ctx.clone());
        let mut data = AppData::load();
        // verify existence for the "missing" badge
        for p in &mut data.pdfs {
            p.exists = std::path::Path::new(&p.path).exists();
        }
        Self {
            worker,
            engine_err: None,
            data,
            view: View::Library,
            filter: Filter::All,
            search: String::new(),
            covers: HashMap::new(),
            reader: None,
            show_new_topic: false,
            show_new_tag: false,
            edit_name: String::new(),
            edit_color: PALETTE[0].to_string(),
            toast: None,
        }
    }

    fn save(&self) {
        self.data.save();
    }

    fn toast(&mut self, ctx: &egui::Context, msg: &str) {
        self.toast = Some((msg.to_string(), ctx.input(|i| i.time) + 2.2));
    }

    fn filtered_pdfs(&self) -> Vec<PdfEntry> {
        let q = self.search.to_lowercase();
        self.data
            .pdfs
            .iter()
            .filter(|p| match &self.filter {
                Filter::All => true,
                Filter::Unsorted => p.topic_id.is_none(),
                Filter::Topic(id) => p.topic_id.as_deref() == Some(id.as_str()),
                Filter::Tag(id) => p.tag_ids.iter().any(|t| t == id),
            })
            .filter(|p| q.is_empty() || p.name.to_lowercase().contains(&q))
            .cloned()
            .collect()
    }

    fn add_pdfs(&mut self, ctx: &egui::Context, paths: Vec<std::path::PathBuf>) {
        let mut added = 0;
        for path in paths {
            let ps = path.to_string_lossy().to_string();
            if self.data.pdfs.iter().any(|p| p.path == ps) {
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
            let topic_id = match &self.filter {
                Filter::Topic(id) => Some(id.clone()),
                _ => None,
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
                topic_id,
                tag_ids,
                exists: true,
            });
            added += 1;
        }
        if added > 0 {
            self.save();
            self.toast(ctx, &format!("Added {added} PDF{}", if added == 1 { "" } else { "s" }));
        }
    }

    fn open_reader(&mut self, pdf: &PdfEntry) {
        if !pdf.exists {
            self.engine_err = Some(format!("File not found: {}", pdf.path));
            return;
        }
        // Ask the worker to (re)open the document; pages/chars stream in later.
        self.worker.send(Req::Close);
        self.worker.send(Req::Open(pdf.path.clone()));
        self.reader = Some(Reader {
            pdf_id: pdf.id.clone(),
            path: pdf.path.clone(),
            name: pdf.name.clone(),
            page_count: 0,
            default_size: (595.0, 842.0),
            zoom: 1.4,
            pages: Vec::new(),
            current_page: 1,
            hl_color: HL_COLORS[0].to_string(),
            selection: None,
            open_failed: None,
            scroll_offset: 0.0,
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
        // absorb everything the worker produced since the last frame
        for res in self.worker.drain() {
            self.handle_res(ctx, res);
        }

        // top bar
        egui::TopBottomPanel::top("topbar")
            .exact_height(44.0)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(14, 0)))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(egui::RichText::new("Folio").color(ACCENT).size(18.0).strong());
                    ui.add_space(8.0);
                    match &self.view {
                        View::Library => {
                            ui.label(egui::RichText::new("· Library").color(TXT_DIM).size(13.0));
                        }
                        View::Reader => {
                            if ui.button("‹  Library").clicked() {
                                self.view = View::Library;
                            }
                            if let Some(r) = &self.reader {
                                ui.add_space(6.0);
                                ui.label(egui::RichText::new(&r.name).color(TXT_DIM).size(13.0));
                            }
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new("v0.6 · native").color(TXT_DIM).size(11.0));
                    });
                });
            });

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
                            .fill(CARD)
                            .stroke(Stroke::new(1.0_f32, BORDER))
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(egui::Margin::symmetric(14, 8))
                            .show(ui, |ui| {
                                ui.label(egui::RichText::new(msg).color(TXT));
                            });
                    });
                ctx.request_repaint();
            } else {
                self.toast = None;
            }
        }
    }
}

impl Folio {
    // ── Library ─────────────────────────────────────────────────────────────
    fn ui_library(&mut self, ctx: &egui::Context) {
        // sidebar
        egui::SidePanel::left("sidebar")
            .exact_width(238.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::same(0)))
            .show(ctx, |ui| self.ui_sidebar(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::same(16)))
            .show(ctx, |ui| {
                if let Some(err) = &self.engine_err {
                    ui.colored_label(Color32::from_rgb(0xd9, 0x65, 0x65), err);
                }
                // toolbar
                ui.horizontal(|ui| {
                    let title = match &self.filter {
                        Filter::All => "All PDFs".to_string(),
                        Filter::Unsorted => "Unsorted".to_string(),
                        Filter::Topic(id) => self.data.topic(id).map(|t| t.name.clone()).unwrap_or_default(),
                        Filter::Tag(id) => self.data.tag(id).map(|t| t.name.clone()).unwrap_or_default(),
                    };
                    ui.heading(egui::RichText::new(title).color(TXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new(egui::RichText::new("+  Add PDFs").color(Color32::BLACK)).fill(ACCENT)).clicked() {
                            if let Some(files) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_files() {
                                self.add_pdfs(ctx, files);
                            }
                        }
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new("Search:").color(TXT_DIM).size(12.0));
                        ui.add(egui::TextEdit::singleline(&mut self.search).desired_width(160.0).hint_text("name…"));
                    });
                });
                ui.add_space(10.0);

                let pdfs = self.filtered_pdfs();
                if pdfs.is_empty() {
                    ui.add_space(60.0);
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("No PDFs here yet").color(TXT_DIM).size(15.0));
                        ui.label(egui::RichText::new("Click “Add PDFs” to get started").color(TXT_DIM).size(12.0));
                    });
                    return;
                }

                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    self.ui_grid(ui, ctx, &pdfs);
                });
            });
    }

    fn ui_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 0)).show(ui, |ui| {
            // filter rows
            let all_n = self.data.pdfs.len();
            self.sidebar_row(ui, "All PDFs", Color32::from_rgb(0x5e, 0x5c, 0x59), all_n, matches!(self.filter, Filter::All), Filter::All);
            let uns = self.data.pdfs.iter().filter(|p| p.topic_id.is_none()).count();
            if uns > 0 {
                self.sidebar_row(ui, "Unsorted", Color32::from_rgb(0x3e, 0x3c, 0x39), uns, matches!(self.filter, Filter::Unsorted), Filter::Unsorted);
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("TOPICS").color(TXT_DIM).size(10.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("+").clicked() {
                        self.show_new_topic = true;
                        self.edit_name.clear();
                        self.edit_color = PALETTE[0].to_string();
                    }
                });
            });
            let topics = self.data.topics.clone();
            for t in &topics {
                let n = self.data.pdfs.iter().filter(|p| p.topic_id.as_deref() == Some(t.id.as_str())).count();
                let active = matches!(&self.filter, Filter::Topic(id) if id == &t.id);
                let resp = self.sidebar_row(ui, &t.name, parse_hex(&t.color), n, active, Filter::Topic(t.id.clone()));
                resp.context_menu(|ui| {
                    if ui.button("Delete topic").clicked() {
                        self.data.pdfs.iter_mut().for_each(|p| {
                            if p.topic_id.as_deref() == Some(t.id.as_str()) { p.topic_id = None; }
                        });
                        self.data.topics.retain(|x| x.id != t.id);
                        if matches!(&self.filter, Filter::Topic(id) if id == &t.id) { self.filter = Filter::All; }
                        self.save();
                        ui.close_menu();
                    }
                });
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("TAGS").color(TXT_DIM).size(10.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("+").clicked() {
                        self.show_new_tag = true;
                        self.edit_name.clear();
                        self.edit_color = PALETTE[1].to_string();
                    }
                });
            });
            let tags = self.data.tags.clone();
            ui.horizontal_wrapped(|ui| {
                for t in &tags {
                    let active = matches!(&self.filter, Filter::Tag(id) if id == &t.id);
                    let col = parse_hex(&t.color);
                    let txt = egui::RichText::new(format!("● {}", t.name)).color(if active { col } else { TXT_DIM }).size(12.0);
                    let resp = ui.add(egui::Button::new(txt).fill(if active { CARD_HOV } else { CARD }).stroke(Stroke::new(1.0_f32, if active { col } else { BORDER })));
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
                    ui.label(egui::RichText::new("No tags yet").color(TXT_DIM).size(11.0));
                }
            });
        });

        // footer with export/import
        egui::TopBottomPanel::bottom("sb-foot")
            .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::same(10)))
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
                ui.label(egui::RichText::new(format!("{} PDFs · {} topics · {} tags", self.data.pdfs.len(), self.data.topics.len(), self.data.tags.len())).color(TXT_DIM).size(11.0));
            });
    }

    fn sidebar_row(&mut self, ui: &mut egui::Ui, label: &str, dot: Color32, count: usize, active: bool, target: Filter) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click());
        let p = ui.painter();
        if active {
            p.rect_filled(rect, CornerRadius::same(6), CARD_HOV);
        } else if resp.hovered() {
            p.rect_filled(rect, CornerRadius::same(6), CARD);
        }
        let cy = rect.center().y;
        p.circle_filled(Pos2::new(rect.left() + 12.0, cy), 4.0, dot);
        p.text(Pos2::new(rect.left() + 24.0, cy), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(13.0), if active { TXT } else { TXT_DIM });
        p.text(Pos2::new(rect.right() - 8.0, cy), egui::Align2::RIGHT_CENTER, count.to_string(), egui::FontId::proportional(11.0), TXT_DIM);
        if resp.clicked() {
            self.filter = target;
        }
        resp
    }

    fn ui_grid(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, pdfs: &[PdfEntry]) {
        let card_w = 156.0;
        let cover_h = card_w / 0.707;
        let spacing = 14.0;
        let avail = ui.available_width();
        let cols = ((avail + spacing) / (card_w + spacing)).floor().max(1.0) as usize;

        egui::Grid::new("pdf-grid").spacing(Vec2::new(spacing, spacing)).show(ui, |ui| {
            for (i, pdf) in pdfs.iter().enumerate() {
                self.ui_card(ui, ctx, pdf, card_w, cover_h);
                if (i + 1) % cols == 0 {
                    ui.end_row();
                }
            }
        });
    }

    fn ui_card(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context, pdf: &PdfEntry, w: f32, cover_h: f32) {
        let total_h = cover_h + 46.0;
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, total_h), Sense::click());
        let p = ui.painter().clone();
        let hovered = resp.hovered();
        p.rect_filled(rect, CornerRadius::same(10), if hovered { CARD_HOV } else { CARD });
        p.rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0_f32, if hovered { parse_hex("#3a3a42") } else { BORDER }), egui::StrokeKind::Inside);

        let cover_rect = Rect::from_min_size(rect.min, Vec2::new(w, cover_h));

        // ensure cover
        self.ensure_cover(pdf);
        match self.covers.get(&pdf.id) {
            Some(Cover::Ready(tex)) => {
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                p.image(tex.id(), cover_rect.shrink(1.0), uv, Color32::WHITE);
            }
            Some(Cover::Failed) => {
                p.text(cover_rect.center(), egui::Align2::CENTER_CENTER, "PDF", egui::FontId::proportional(20.0), TXT_DIM);
            }
            _ => {
                p.text(cover_rect.center(), egui::Align2::CENTER_CENTER, "…", egui::FontId::proportional(20.0), TXT_DIM);
            }
        }

        // name + meta
        let name_pos = Pos2::new(rect.left() + 9.0, cover_rect.bottom() + 8.0);
        let name = elide(&pdf.name, 22);
        p.text(name_pos, egui::Align2::LEFT_TOP, name, egui::FontId::proportional(12.0), TXT);
        let meta = if let Some(t) = pdf.topic_id.as_ref().and_then(|id| self.data.topic(id)) {
            format!("{} · {}", fmt_size(pdf.size), t.name)
        } else {
            fmt_size(pdf.size)
        };
        p.text(Pos2::new(rect.left() + 9.0, cover_rect.bottom() + 26.0), egui::Align2::LEFT_TOP, elide(&meta, 26), egui::FontId::proportional(10.5), TXT_DIM);

        if !pdf.exists {
            p.text(Pos2::new(rect.left() + 6.0, rect.top() + 6.0), egui::Align2::LEFT_TOP, "⚠ missing", egui::FontId::proportional(10.0), Color32::from_rgb(0xd9, 0x65, 0x65));
        }

        if resp.clicked() {
            self.open_reader(pdf);
        }
        resp.context_menu(|ui| self.card_menu(ui, pdf));
    }

    fn card_menu(&mut self, ui: &mut egui::Ui, pdf: &PdfEntry) {
        if ui.button("Open").clicked() {
            self.open_reader(pdf);
            ui.close_menu();
        }
        ui.separator();
        ui.label(egui::RichText::new("Topic").color(TXT_DIM).size(10.0));
        let topics = self.data.topics.clone();
        let cur = pdf.topic_id.clone();
        if ui.selectable_label(cur.is_none(), "None").clicked() {
            if let Some(p) = self.data.pdfs.iter_mut().find(|p| p.id == pdf.id) { p.topic_id = None; }
            self.save();
            ui.close_menu();
        }
        for t in &topics {
            if ui.selectable_label(cur.as_deref() == Some(t.id.as_str()), &t.name).clicked() {
                if let Some(p) = self.data.pdfs.iter_mut().find(|p| p.id == pdf.id) { p.topic_id = Some(t.id.clone()); }
                self.save();
                ui.close_menu();
            }
        }
        ui.separator();
        ui.label(egui::RichText::new("Tags").color(TXT_DIM).size(10.0));
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
        if self.show_new_topic {
            let mut open = true;
            let mut create = false;
            egui::Window::new("New Topic").collapsible(false).resizable(false).open(&mut open).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.edit_name).hint_text("Topic name"));
                ui.add_space(6.0);
                color_picker(ui, &mut self.edit_color);
                ui.add_space(10.0);
                if ui.add(egui::Button::new(egui::RichText::new("Create").color(Color32::BLACK)).fill(ACCENT)).clicked() {
                    create = true;
                }
            });
            if create && !self.edit_name.trim().is_empty() {
                self.data.topics.push(Topic { id: uuid::Uuid::new_v4().to_string(), name: self.edit_name.trim().to_string(), color: self.edit_color.clone() });
                self.save();
                self.show_new_topic = false;
            }
            if !open { self.show_new_topic = false; }
        }

        if self.show_new_tag {
            let mut open = true;
            let mut create = false;
            egui::Window::new("New Tag").collapsible(false).resizable(false).open(&mut open).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.edit_name).hint_text("Tag name"));
                ui.add_space(6.0);
                color_picker(ui, &mut self.edit_color);
                ui.add_space(10.0);
                if ui.add(egui::Button::new(egui::RichText::new("Create").color(Color32::BLACK)).fill(ACCENT)).clicked() {
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
        // toolbar
        let mut zoom_delta = 0.0f32;
        let mut do_copy = false;
        egui::TopBottomPanel::top("reader-tb")
            .exact_height(40.0)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(10, 0)))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    if let Some(r) = &mut self.reader {
                        let total = if r.page_count == 0 { "…".to_string() } else { r.page_count.to_string() };
                        ui.label(egui::RichText::new(format!("Page {} / {}", r.current_page, total)).color(TXT_DIM).size(12.0));
                        ui.separator();
                        if ui.button(" −  ").clicked() { zoom_delta = -0.2; }
                        ui.label(egui::RichText::new(format!("{}%", (r.zoom * 100.0).round() as i32)).color(TXT_DIM).size(12.0));
                        if ui.button(" +  ").clicked() { zoom_delta = 0.2; }
                        ui.separator();
                        ui.label(egui::RichText::new("Highlight:").color(TXT_DIM).size(12.0));
                        for c in HL_COLORS {
                            let sel = &r.hl_color == c;
                            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
                            ui.painter().circle_filled(rect.center(), 8.0, parse_hex(c));
                            if sel {
                                ui.painter().circle_stroke(rect.center(), 9.0, Stroke::new(2.0_f32, TXT));
                            }
                            if resp.clicked() { r.hl_color = c.to_string(); }
                        }
                        if r.selection.as_ref().map(|s| !s.text.is_empty()).unwrap_or(false) {
                            ui.separator();
                            if ui.button("Copy").clicked() { do_copy = true; }
                        }
                    }
                });
            });

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

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(PAGE_BG))
            .show(ctx, |ui| {
                self.ui_pages(ui, ctx);
            });
    }

    fn ui_pages(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        // Ctrl/⌘ + scroll (or trackpad pinch) zooms, anchored under the cursor —
        // must run before we snapshot `zoom`, and it may force a scroll offset.
        let viewport_top = ui.max_rect().top();
        let forced_offset = self.apply_scroll_zoom(ctx, viewport_top);

        let Some(reader) = self.reader.as_ref() else { return };

        if let Some(err) = &reader.open_failed {
            ui.centered_and_justified(|ui| {
                ui.colored_label(Color32::from_rgb(0xd9, 0x65, 0x65), format!("Could not open document:\n{err}"));
            });
            return;
        }
        if reader.page_count == 0 {
            ui.centered_and_justified(|ui| {
                ui.label(egui::RichText::new("Opening…").color(TXT_DIM).size(14.0));
            });
            ctx.request_repaint();
            return;
        }

        let zoom = reader.zoom;
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
        let mut deselect = false; // a plain click on empty text clears the selection
        let mut requests: Vec<Req> = Vec::new();
        let mut top_visible_page = reader.current_page;
        let mut first_visible_set = false;

        let mut area = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .drag_to_scroll(false); // drags are for text selection, not panning
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

                        let (rect, resp) = ui.allocate_exact_size(disp, Sense::click_and_drag());
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
                                p.text(rect.center(), egui::Align2::CENTER_CENTER, "…", egui::FontId::proportional(18.0), TXT_DIM);
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
                                        let pad_x = 1.0;
                                        for (x0, y0, x1, y1) in selection_rects(chars, lines, sel.start, sel.end) {
                                            let sr = Rect::from_min_max(
                                                Pos2::new(rect.left() + x0 * zoom - pad_x, rect.top() + y0 * zoom),
                                                Pos2::new(rect.left() + x1 * zoom + pad_x, rect.top() + y1 * zoom),
                                            );
                                            p.rect_filled(sr, CornerRadius::same(2), SELECT_FILL);
                                        }
                                    }
                                }
                            }

                            // text interaction: I-beam cursor, drag-select, word-select, hit-testing
                            if let (Some(chars), Some(lines)) = (&slot.chars, &slot.lines) {
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
                                    // A plain click removes a highlight, or clears the selection.
                                    if let Some(pos) = resp.interact_pointer_pos() {
                                        match over_highlight(pos) {
                                            Some(id) => remove_highlight = Some(id),
                                            None => deselect = true,
                                        }
                                    }
                                }

                                // finished a drag → materialise selection text
                                if resp.drag_stopped() {
                                    if let Some(cur) = &reader.selection {
                                        if cur.page == idx + 1 {
                                            let text = selection_text(chars, cur.start, cur.end);
                                            new_selection = Some(Selection { page: cur.page, start: cur.start, end: cur.end, text });
                                        }
                                    }
                                }
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
        } else if deselect {
            reader.selection = None;
        }
        reader.current_page = top_visible_page;

        // Floating "Highlight selection" button. Snapshot the selection first so
        // the closure never borrows `reader` (avoids aliasing).
        let sel_snapshot = reader.selection.as_ref().filter(|s| !s.text.is_empty()).map(|s| {
            let page_idx = (s.page - 1) as usize;
            (
                s.page,
                s.start,
                s.end,
                reader.pages[page_idx].chars.clone(),
                reader.pages[page_idx].lines.clone(),
                reader.pages[page_idx].size_pts.unwrap_or(default_size),
            )
        });
        let mut clear_selection = false;
        if let Some((page, start, end, chars, lines, (w_pts, h_pts))) = sel_snapshot {
            egui::Area::new(egui::Id::new("hl-fab")).anchor(egui::Align2::RIGHT_BOTTOM, [-20.0, -20.0]).show(ctx, |ui| {
                egui::Frame::new().fill(CARD).stroke(Stroke::new(1.0_f32, BORDER)).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::new(egui::RichText::new("Highlight selection").color(Color32::BLACK)).fill(parse_hex(&hl_color))).clicked() {
                            if let (Some(chars), Some(lines)) = (&chars, &lines) {
                                let rects = selection_rects(chars, lines, start, end).into_iter().map(|(x0, y0, x1, y1)| HighlightRect {
                                    x: x0 / w_pts, y: y0 / h_pts, w: (x1 - x0) / w_pts, h: (y1 - y0) / h_pts,
                                }).collect();
                                save_highlight = Some(PdfHighlight { id: uuid::Uuid::new_v4().to_string(), page, rects, color: hl_color.clone() });
                            }
                        }
                        if ui.button("×").clicked() {
                            clear_selection = true;
                        }
                    });
                });
            });
        }
        if clear_selection {
            reader.selection = None;
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

fn setup_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(TXT);
    visuals.panel_fill = BG;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = CARD;
    visuals.widgets.inactive.bg_fill = CARD;
    visuals.widgets.hovered.bg_fill = CARD_HOV;
    visuals.widgets.active.bg_fill = CARD_HOV;
    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    ctx.set_visuals(visuals);
}

fn upload(ctx: &egui::Context, name: &str, img: ColorImage) -> TextureHandle {
    ctx.load_texture(name, img, egui::TextureOptions::LINEAR)
}

fn color_picker(ui: &mut egui::Ui, current: &mut String) {
    ui.horizontal_wrapped(|ui| {
        for c in PALETTE {
            let sel = current == c;
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
            ui.painter().circle_filled(rect.center(), 11.0, parse_hex(c));
            if sel {
                ui.painter().circle_stroke(rect.center(), 12.0, Stroke::new(2.0_f32, TXT));
            }
            if resp.clicked() {
                *current = c.to_string();
            }
        }
    });
}

/// Group glyphs (in reading order) into text lines the way Poppler/Okular do:
/// a new line starts when a glyph no longer shares the current line's baseline
/// (lower-y) and height. `chars` are assumed left-to-right within a line.
fn build_lines(chars: &[CharBox]) -> Vec<LineBox> {
    let mut lines: Vec<LineBox> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        let mut top = chars[i].y;
        let mut bottom = chars[i].y + chars[i].h;
        let mut base = bottom; // baseline proxy: box bottom
        let mut height = chars[i].h;
        i += 1;
        while i < chars.len() {
            let c = &chars[i];
            let c_base = c.y + c.h;
            let tol = height.max(c.h) * 0.5;
            let same_baseline = (c_base - base).abs() <= tol;
            let similar_height = (c.h - height).abs() <= height.max(c.h) * 0.6;
            if same_baseline && similar_height {
                top = top.min(c.y);
                bottom = bottom.max(c_base);
                // track the running line metrics off the latest glyph
                base = c_base;
                height = height.max(c.h);
                i += 1;
            } else {
                break;
            }
        }
        lines.push(LineBox { start, end: i, top, bottom });
    }
    lines
}

/// Index of the line nearest a vertical page position (containing it if possible).
fn line_at(lines: &[LineBox], py: f32) -> Option<usize> {
    if lines.is_empty() {
        return None;
    }
    let mut best = 0usize;
    let mut best_d = f32::MAX;
    for (k, l) in lines.iter().enumerate() {
        let d = if py < l.top {
            l.top - py
        } else if py > l.bottom {
            py - l.bottom
        } else {
            0.0
        };
        if d < best_d {
            best_d = d;
            best = k;
        }
        if d == 0.0 {
            break;
        }
    }
    Some(best)
}

/// Nearest glyph to a page-point position, restricted to its line (word/line select).
fn char_at(chars: &[CharBox], lines: &[LineBox], px: f32, py: f32) -> Option<usize> {
    let li = line_at(lines, py)?;
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

/// Caret index (0..=len) nearest a page-point position. Snaps to the nearest
/// line, then to the glyph boundary nearest the pointer within that line —
/// dragging past the last glyph selects to the line's end, like a real editor.
fn caret_at(chars: &[CharBox], lines: &[LineBox], px: f32, py: f32) -> usize {
    let Some(li) = line_at(lines, py) else { return 0 };
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
        let mut x0 = f32::MAX;
        let mut x1 = f32::MIN;
        for c in &chars[s..e] {
            x0 = x0.min(c.x);
            x1 = x1.max(c.x + c.w);
        }
        if x1 > x0 {
            rects.push((x0, l.top, x1, l.bottom));
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

fn elide(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    } else {
        s.to_string()
    }
}
