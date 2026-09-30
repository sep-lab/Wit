//! The tray: a real popover window (not a native menu), so it can show
//! the same Svelte `TrayPopover.svelte` the main window would — one line
//! per song, no badges, no notifications (this lane's brief, item 4). On
//! a Linux desktop without an AppIndicator implementation, the OS simply
//! never shows the tray icon; Wit still opens as a normal window (the
//! main window is created and shown regardless of tray setup).

use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App, AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_positioner::{Position, WindowExt};

pub const TRAY_WINDOW: &str = "tray-popover";
pub const MAIN_WINDOW: &str = "main";

/// A monochrome circle, fully transparent outside it — a template icon
/// (review round 1, blocking #5). macOS ignores an `icon_as_template`
/// image's colour entirely and uses only its alpha channel, recolouring
/// the opaque pixels to match the current menu-bar appearance; the app's
/// own icon (`icons/icon.png`, used for `default_window_icon()`) has an
/// *opaque* background, which under template mode would fill the whole
/// status-item slot with a solid black square instead of drawing a glyph.
///
/// Raw decoded RGBA8, not a PNG: `tauri::image::Image` takes pixels
/// directly (`Image::new`), so there's no need for a PNG-decoding
/// dependency just for one small generated icon.
const TRAY_ICON_SIZE: u32 = 44;
const TRAY_ICON_RGBA: &[u8] = include_bytes!("../icons/tray-icon.rgba");

fn tray_icon() -> Image<'static> {
    Image::new(TRAY_ICON_RGBA, TRAY_ICON_SIZE, TRAY_ICON_SIZE)
}

/// Build the "main" window fresh, with the same shape `tauri.conf.json`'s
/// `app.windows[0]` describes. Used both at startup (implicitly, by Tauri
/// itself, from that config) and defensively by `show_main_window` if the
/// window was ever destroyed rather than hidden (review round 1, blocking
/// #5: today, closing it destroys it while the hidden tray popover keeps
/// the process alive, and there is no way back).
pub fn ensure_main_window(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        return Ok(window);
    }
    WebviewWindowBuilder::new(app, MAIN_WINDOW, WebviewUrl::App("index.html".into()))
        .title("Wit")
        .inner_size(960.0, 720.0)
        .min_inner_size(480.0, 480.0)
        .build()
}

pub fn setup(app: &App) -> tauri::Result<()> {
    // Closing the main window hides it (the app keeps running in the
    // tray) instead of destroying it — Quit is still the tray menu's job.
    if let Some(main) = app.get_webview_window(MAIN_WINDOW) {
        let hide_instead_of_close = main.clone();
        main.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = hide_instead_of_close.hide();
            }
        });
    }

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
        .icon(tray_icon())
        .icon_as_template(true)
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
