//! Tiny hand-drawn vector icon set, rendered with the egui painter so they stay
//! crisp at any size and need no font/emoji glyphs or external assets. Style:
//! sober single-weight line icons on a square grid, à la a modern line set.

use eframe::egui::{self, Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    Library,  // all documents
    Inbox,    // unsorted
    Folder,   // a topic
    Tag,      // a tag
    Contents, // table of contents / list
    DotsV,    // three vertical dots (overflow menu)
    Grid,     // flat grid view
    Groups,   // grouped-by-topic view
    Search,   // magnifier
    Plus,     // add
    Chevron,  // collapse indicator (points down when open)
    ChevronRight,
    Sidebar,  // panel toggle
    Hand,     // pan / drag tool
    Cursor,   // text-selection I-beam
    Marker,   // highlighter
    Account,  // signed-in user
}

/// Draw `icon` centred in `rect`, tinted `color`. The icon is inscribed in the
/// largest centred square that fits, with a small inset for optical balance.
pub fn draw(p: &Painter, icon: Icon, rect: Rect, color: Color32) {
    let side = rect.width().min(rect.height());
    let sq = Rect::from_center_size(rect.center(), Vec2::splat(side)).shrink(side * 0.14);
    let w = (side * 0.08).clamp(1.2, 2.4); // stroke weight scales with size
    let s = Stroke::new(w, color);
    // Map normalised (0..1, 0..1) coordinates into the icon square.
    let pt = |nx: f32, ny: f32| Pos2::new(sq.left() + nx * sq.width(), sq.top() + ny * sq.height());
    let line = |a: Pos2, b: Pos2| p.line_segment([a, b], s);
    let poly = |pts: Vec<Pos2>| p.add(Shape::closed_line(pts, s));

    match icon {
        Icon::Library => {
            // three stacked "sheets"
            for (i, dy) in [0.0f32, 0.22, 0.44].iter().enumerate() {
                let y = 0.16 + dy;
                let inset = 0.0 + i as f32 * 0.0;
                p.add(Shape::line(
                    vec![pt(0.16 + inset, y + 0.12), pt(0.16 + inset, y), pt(0.84 - inset, y), pt(0.84 - inset, y + 0.12)],
                    s,
                ));
            }
        }
        Icon::Inbox => {
            poly(vec![pt(0.16, 0.2), pt(0.84, 0.2), pt(0.84, 0.8), pt(0.16, 0.8)]);
            line(pt(0.16, 0.56), pt(0.36, 0.56));
            line(pt(0.36, 0.56), pt(0.42, 0.68));
            line(pt(0.42, 0.68), pt(0.58, 0.68));
            line(pt(0.58, 0.68), pt(0.64, 0.56));
            line(pt(0.64, 0.56), pt(0.84, 0.56));
        }
        Icon::Folder => {
            p.add(Shape::line(
                vec![
                    pt(0.14, 0.78),
                    pt(0.14, 0.28),
                    pt(0.42, 0.28),
                    pt(0.5, 0.4),
                    pt(0.86, 0.4),
                    pt(0.86, 0.78),
                    pt(0.14, 0.78),
                ],
                s,
            ));
        }
        Icon::Tag => {
            poly(vec![pt(0.2, 0.2), pt(0.55, 0.2), pt(0.82, 0.47), pt(0.47, 0.82), pt(0.2, 0.55)]);
            p.circle(pt(0.36, 0.36), w * 0.9, color, Stroke::NONE);
        }
        Icon::Contents => {
            for (i, y) in [0.26f32, 0.5, 0.74].iter().enumerate() {
                let bullet = 0.2 + i as f32 * 0.0;
                p.circle_filled(pt(0.2, *y), w * 0.7, color);
                line(pt(0.34 + bullet - 0.2, *y), pt(0.84, *y));
            }
        }
        Icon::DotsV => {
            for y in [0.24f32, 0.5, 0.76] {
                p.circle_filled(pt(0.5, y), w * 1.05, color);
            }
        }
        Icon::Grid => {
            for (cx, cy) in [(0.32f32, 0.32f32), (0.68, 0.32), (0.32, 0.68), (0.68, 0.68)] {
                let c = pt(cx, cy);
                let r = Rect::from_center_size(c, Vec2::splat(sq.width() * 0.26));
                p.rect_stroke(r, egui::CornerRadius::same(2), s, egui::StrokeKind::Middle);
            }
        }
        Icon::Groups => {
            // two little folders side by side
            for dx in [0.0f32, 0.0] {
                let _ = dx;
            }
            p.add(Shape::line(vec![pt(0.12, 0.7), pt(0.12, 0.34), pt(0.32, 0.34), pt(0.38, 0.44), pt(0.52, 0.44), pt(0.52, 0.7), pt(0.12, 0.7)], s));
            p.add(Shape::line(vec![pt(0.46, 0.78), pt(0.46, 0.5), pt(0.64, 0.5), pt(0.7, 0.58), pt(0.88, 0.58), pt(0.88, 0.78), pt(0.46, 0.78)], s));
        }
        Icon::Search => {
            let c = pt(0.44, 0.44);
            p.circle_stroke(c, sq.width() * 0.24, s);
            line(pt(0.62, 0.62), pt(0.82, 0.82));
        }
        Icon::Plus => {
            line(pt(0.5, 0.22), pt(0.5, 0.78));
            line(pt(0.22, 0.5), pt(0.78, 0.5));
        }
        Icon::Chevron => {
            line(pt(0.28, 0.4), pt(0.5, 0.62));
            line(pt(0.5, 0.62), pt(0.72, 0.4));
        }
        Icon::ChevronRight => {
            line(pt(0.4, 0.28), pt(0.62, 0.5));
            line(pt(0.62, 0.5), pt(0.4, 0.72));
        }
        Icon::Sidebar => {
            p.rect_stroke(Rect::from_min_max(pt(0.16, 0.22), pt(0.84, 0.78)), egui::CornerRadius::same(2), s, egui::StrokeKind::Middle);
            line(pt(0.42, 0.22), pt(0.42, 0.78));
        }
        Icon::Hand => {
            // An open hand for "drag / move the page": the palm is an open cup
            // (sides + rounded bottom, no lid) so the fingers reading above it
            // don't merge into a solid box, plus a thumb to one side.
            p.add(Shape::line(
                vec![
                    pt(0.30, 0.40),
                    pt(0.30, 0.66),
                    pt(0.36, 0.78),
                    pt(0.64, 0.78),
                    pt(0.70, 0.66),
                    pt(0.70, 0.40),
                ],
                s,
            ));
            for x in [0.38f32, 0.50, 0.62] {
                line(pt(x, 0.44), pt(x, 0.22)); // three fingers
            }
            line(pt(0.30, 0.52), pt(0.18, 0.44)); // thumb
        }
        Icon::Cursor => {
            // Text I-beam.
            line(pt(0.5, 0.22), pt(0.5, 0.78));
            line(pt(0.4, 0.22), pt(0.6, 0.22));
            line(pt(0.4, 0.78), pt(0.6, 0.78));
        }
        Icon::Marker => {
            // Three lines of text with the middle one emphasised — the ISOTYPE
            // reading of "select text and highlight it". Pairs with the I-beam.
            line(pt(0.2, 0.30), pt(0.8, 0.30));
            p.line_segment([pt(0.2, 0.5), pt(0.8, 0.5)], Stroke::new(w * 2.6, color)); // highlighted
            line(pt(0.2, 0.70), pt(0.66, 0.70));
        }
        Icon::Account => {
            // A person: head circle above a shoulders arc.
            p.circle_stroke(pt(0.5, 0.34), sq.width() * 0.155, s);
            let n = 18;
            let arc: Vec<Pos2> = (0..=n)
                .map(|i| {
                    let a = std::f32::consts::PI * (1.0 + i as f32 / n as f32); // 180°..360°
                    pt(0.5 + 0.30 * a.cos(), 0.94 + 0.30 * a.sin())
                })
                .collect();
            p.add(Shape::line(arc, s));
        }
    }
}

/// The Google Drive triangular mark, filled. `colored` draws it in Drive's
/// blue/green/yellow; otherwise it is rendered in greys to signal a
/// disconnected state.
pub fn draw_drive(p: &Painter, rect: Rect, colored: bool) {
    let side = rect.width().min(rect.height());
    let sq = Rect::from_center_size(rect.center(), Vec2::splat(side));
    let pt = |nx: f32, ny: f32| Pos2::new(sq.left() + nx * sq.width(), sq.top() + ny * sq.height());
    let (t, l, r, c) = (pt(0.5, 0.13), pt(0.08, 0.87), pt(0.92, 0.87), pt(0.5, 0.62));
    let (blue, green, yellow) = if colored {
        (
            Color32::from_rgb(0x26, 0x84, 0xfc),
            Color32::from_rgb(0x00, 0xac, 0x47),
            Color32::from_rgb(0xff, 0xba, 0x00),
        )
    } else {
        (Color32::from_gray(0x66), Color32::from_gray(0x86), Color32::from_gray(0x9c))
    };
    let tri = |a: Pos2, b: Pos2, d: Pos2, col: Color32| {
        p.add(Shape::convex_polygon(vec![a, b, d], col, Stroke::NONE));
    };
    tri(l, r, c, blue); // bottom
    tri(l, t, c, yellow); // left
    tri(r, t, c, green); // right
}

/// Allocate a clickable, icon-only button of `size` px. Returns the response.
pub fn button(ui: &mut egui::Ui, icon: Icon, size: f32, color: Color32, hover_bg: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(6), hover_bg);
    }
    let tint = if resp.hovered() { color } else { color.gamma_multiply(0.85) };
    draw(ui.painter(), icon, rect.shrink(size * 0.2), tint);
    resp
}

/// Icon-only toggle button. When `active`, it is drawn with a persistent
/// highlighted background so the selected tool reads at a glance (ISOTYPE:
/// the state is carried by the pictogram, not a text label).
pub fn toggle(
    ui: &mut egui::Ui,
    icon: Icon,
    size: f32,
    color: Color32,
    active_bg: Color32,
    hover_bg: Color32,
    active: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click());
    if active {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(6), active_bg);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(6), hover_bg);
    }
    let tint = if active || resp.hovered() { color } else { color.gamma_multiply(0.85) };
    draw(ui.painter(), icon, rect.shrink(size * 0.2), tint);
    resp
}
