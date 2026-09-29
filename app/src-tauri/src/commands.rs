use std::path::{Path, PathBuf};

use tauri::{command, AppHandle, Manager};
use wit_story::{Clock, Comparison, Library, Timestamp};

use crate::error::NotAvailable;
use crate::fixture::demo_library;

/// The clock the OS gives us: now, plus the viewer's local UTC offset —
/// this lane's brief, item 3. Never a network time source (ADR-0008).
fn local_clock() -> Clock {
    let now = Timestamp(chrono::Utc::now().timestamp());
    // `local_minus_utc()` is seconds and always a whole number of minutes.
    let offset_minutes = chrono::Local::now().offset().local_minus_utc() / 60;
    Clock::fixed(now, offset_minutes)
}

/// The folder as the owner sees it, with `~` for the home directory —
/// this lane's brief, item 3 ("label = the folder shown with `~` for the
/// home dir"). Falls back to the raw path when it isn't under the home
/// directory, or the home directory can't be found.
fn folder_label(root: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rel) = root.strip_prefix(&home) {
            return if rel.as_os_str().is_empty() {
                "~".to_string()
            } else {
                format!("~/{}", rel.display())
            };
        }
    }
    root.display().to_string()
}

/// `wit_story::build_library` for a configured root, or the fixture — this
/// lane's brief, item 3: "With no root, or when the page runs in a plain
/// browser (no Tauri), load `crates/wit-story/fixtures/demo-library.json`."
/// A plain browser has no IPC at all, so this half of that rule is the
/// Tauri side: no root configured (yet) behaves the same way.
#[command]
pub fn library(root: Option<String>) -> Result<Library, String> {
    match root {
        Some(path) => {
            let root_path = PathBuf::from(&path);
            let label = folder_label(&root_path);
            let clock = local_clock();
            Ok(wit_story::build_library(&root_path, &label, &clock))
        }
        None => Ok(demo_library()),
    }
}

/// Drag-compare on the timeline. Stubbed until the engine grows a
/// two-arbitrary-moments compare (this lane's brief, item 3).
#[command]
pub fn compare(story_id: String, from: String, to: String) -> Result<Comparison, NotAvailable> {
    let _ = (story_id, from, to);
    Err(NotAvailable::new("Comparing two moments"))
}

/// "Open this moment as a copy" (ADR-0007). Stubbed until a DAW's
/// restored copy has been shown to open in it.
#[command]
pub fn open_as_copy(story_id: String, moment_id: String) -> Result<(), NotAvailable> {
    let _ = (story_id, moment_id);
    Err(NotAvailable::new("Opening a moment as a copy"))
}

/// "Send to a friend" (Phase E share page). Stubbed until the share page
/// exists.
#[command]
pub fn send(story_id: String, moment_id: String) -> Result<(), NotAvailable> {
    let _ = (story_id, moment_id);
    Err(NotAvailable::new("Sending to a friend"))
}

/// Reveal a song's folder in Finder / Explorer / the file manager.
/// Stubbed until `crates/wit-platform` lands.
#[command]
pub fn reveal(song_id: String) -> Result<(), NotAvailable> {
    let _ = song_id;
    Err(NotAvailable::new("Revealing this song's folder"))
}

/// Show and focus the main window — used by the tray popover's "Open Wit"
/// button. A real command, not a stub: it only needs Tauri's own window
/// API, no engine dependency.
#[command]
pub fn show_main_window(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}
