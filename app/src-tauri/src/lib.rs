mod commands;
mod error;
mod fixture;
mod tray;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
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
