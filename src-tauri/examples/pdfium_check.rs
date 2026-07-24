// Diagnostic: confirm the bundled pdfium shared library loads, initialises, and
// can render + read text at runtime, independent of the GUI. Run with:
//   cargo run --example pdfium_check -- [optional/path/to.pdf]
use pdfium_render::prelude::*;

fn main() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/pdfium");
    let bindings = Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(&dir))
        .expect("failed to bind to libpdfium.so in src-tauri/pdfium");
    let pdfium = Pdfium::new(bindings);
    println!("OK: pdfium bound and initialised from {dir}");

    let Some(path) = std::env::args().nth(1) else {
        println!("(pass a PDF path to also test rendering)");
        return;
    };

    let doc = pdfium.load_pdf_from_file(&path, None).expect("load pdf");
    let pages = doc.pages();
    println!("OK: loaded '{path}' with {} pages", pages.len());

    let page = pages.get(0).expect("page 0");
    let bitmap = page
        .render_with_config(
            &PdfRenderConfig::new()
                .set_target_width(200)
                .set_target_height(280),
        )
        .expect("render");
    let img = bitmap.as_image().into_rgba8();
    println!("OK: rendered page 0 to {}x{} px", img.width(), img.height());

    let text = page.text().expect("text").all();
    let preview: String = text.chars().take(60).collect();
    println!("OK: extracted {} chars. Preview: {:?}", text.len(), preview.trim());
}
