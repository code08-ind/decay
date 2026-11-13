//! Tauri's build step. The app's own commands get permissions (`allow-<command>`), so
//! each window's capability lists exactly what it may call (Software Design §6).

const COMMANDS: &[&str] = &[
    "get_status",
    "set_mode",
    "resume",
    "hold",
    "release_hold",
    "run_sweep_now",
    "undo_sweep",
    "set_hotkey",
    "set_launch_at_login",
    "request_accessibility",
    "ui_show_window",
    "ui_hide_panel",
    "ui_take_pending",
    "get_settings",
    "update_settings",
    "search",
    "restore_items",
    "restore_batch",
    "protect_item",
    "ui_show_item",
];

fn main() {
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    if let Err(e) = tauri_build::try_build(attributes) {
        panic!("tauri build: {e:#}");
    }
}
