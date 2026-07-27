//! Tauri build hook: generates the context for `tauri::generate_context!()`.
//! Required by every Tauri 2 application.

fn main() {
    tauri_build::build()
}
