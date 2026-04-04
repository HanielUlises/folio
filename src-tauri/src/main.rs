// Folio — main.rs
// Tauri entry point. Logic lives in lib.rs.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    folio_lib::run();
}
