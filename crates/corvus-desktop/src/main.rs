//! Thin entry point: everything testable lives in the library.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    corvus_desktop::run(tauri::generate_context!());
}
