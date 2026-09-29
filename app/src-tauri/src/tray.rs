//! The tray: a real popover window (not a native menu), so it can show
//! the same Svelte `TrayPopover.svelte` the main window would — one line
//! per song, no badges, no notifications (this lane's brief, item 4). On
//! a Linux desktop without an AppIndicator implementation, the OS simply
//! never shows the tray icon; Wit still opens as a normal window (the
//! main window is created and shown regardless of tray setup).

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_positioner::{Position, WindowExt};

pub const TRAY_WINDOW: &str = "tray-popover";

pub fn setup(app: &App) -> tauri::Result<()> {
    let popover = WebviewWindowBuilder::new(
        app,
        TRAY_WINDOW,
        WebviewUrl::App("index.html?tray=1".into()),
    )
    .title("Wit")
    .inner_size(280.0, 320.0)
    .resizable(false)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible(false)
    .build()?;

    // Hide the popover when it loses focus, like an ordinary menu-bar
    // popover — clicking elsewhere dismisses it instead of leaving a
    // stray always-on-top window behind.
    let hide_on_blur = popover.clone();
    popover.on_window_event(move |event| {
        if let WindowEvent::Focused(false) = event {
            let _ = hide_on_blur.hide();
        }
    });

    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&quit])?;

    TrayIconBuilder::new()
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            let app = tray.app_handle();
            tauri_plugin_positioner::on_tray_event(app, &event);
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let Some(window) = app.get_webview_window(TRAY_WINDOW) else {
                    return;
                };
                if window.is_visible().unwrap_or(false) {
                    let _ = window.hide();
                } else {
                    let _ = window.move_window(Position::TrayBottomCenter);
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .on_menu_event(|app, event| {
            if event.id() == "quit" {
                app.exit(0);
            }
        })
        .build(app)?;

    Ok(())
}
