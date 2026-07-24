//! Diagnostic: dump how glyphs group into lines for a real PDF page.
//! Run: FOLIO_PDFIUM_DIR=pdfium cargo run --example diag -- "<pdf>" <page0>

use pdfium_render::prelude::*;

#[derive(Clone, Copy)]
struct CharBox {
    ch: char,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

#[derive(Clone, Copy)]
struct LineBox {
    start: usize,
    end: usize,
    top: f32,
    bottom: f32,
    left: f32,
    right: f32,
}

fn char_boxes(page: &PdfPage) -> Vec<CharBox> {
    let page_h = page.height().value;
    let Ok(text) = page.text() else { return Vec::new() };
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
            y: page_h - top,
            w: (right - left).max(0.0),
            h: (top - bottom).max(0.0),
        });
    }
    out
}

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
            let overlap = bottom.min(c_bottom) - top.max(c_top);
            let min_h = (bottom - top).min(c.h).max(1.0);
            if overlap > 0.5 * min_h {
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("pass a pdf path");
    let page_idx: u16 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

    let bindings = Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("pdfium"))
        .or_else(|_| Pdfium::bind_to_system_library())
        .expect("pdfium");
    let pdfium = Pdfium::new(bindings);
    let doc = pdfium.load_pdf_from_file(path, None).expect("open");
    let pages = doc.pages();
    let page = pages.get(page_idx).expect("page");

    let chars = char_boxes(&page);
    let lines = build_lines(&chars);
    println!("page {page_idx}: {} chars, {} lines", chars.len(), lines.len());
    println!("page size: {} x {}", page.width().value, page.height().value);

    for (k, l) in lines.iter().enumerate().take(25) {
        let text: String = chars[l.start..l.end].iter().map(|c| c.ch).collect();
        let n = l.end - l.start;
        let gaps = count_gaps(&chars[l.start..l.end]);
        println!(
            "line {k:2}: [{:4}..{:<4}] n={n:3} y=[{:6.1},{:6.1}] x=[{:6.1},{:6.1}] gaps={gaps} | {:?}",
            l.start, l.end, l.top, l.bottom, l.left, l.right,
            text.chars().take(60).collect::<String>()
        );
    }
}

/// Count horizontal gaps larger than an em-ish width between consecutive glyphs.
fn count_gaps(line: &[CharBox]) -> usize {
    let mut gaps = 0;
    for w in line.windows(2) {
        let a_right = w[0].x + w[0].w;
        let b_left = w[1].x;
        if b_left - a_right > w[0].h * 0.3 {
            gaps += 1;
        }
    }
    gaps
}
