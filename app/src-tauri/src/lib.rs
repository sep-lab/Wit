mod commands;
mod error;
mod fixture;
mod tray;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Registered first, per tauri-plugin-single-instance's own docs:
        // a second launch focuses the first instance instead of spawning
        // a duplicate process (review round 1, non-blocking #11).
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Ok(window) = tray::ensure_main_window(app) {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_positioner::init())
        .invoke_handler(tauri::generate_handler![
            commands::library,
            commands::compare,
            commands::open_as_copy,
            commands::send,
            commands::reveal,
            commands::show_main_window,
        ])
        .setup(|app| {
            tray::setup(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Wit");
}
