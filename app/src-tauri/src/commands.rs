use tauri::{command, AppHandle};
use wit_story::{Comparison, Library};

use crate::error::NotAvailable;
use crate::fixture::demo_library;
use crate::tray::ensure_main_window;

/// The Library to show — the embedded fixture, for now.
///
/// Review round 1, non-blocking #1: this command used to accept an
/// arbitrary root path over IPC (with no caller passing one, and no
/// validation of it) and ran `wit_story::build_library`'s filesystem walk
/// synchronously on the main thread. Both are dropped until
/// `crates/wit-platform` exists to hand this a root it has already
/// checked (never inside a DAW package, never a parent of another
/// watched root, per ADR-0007) and a real OS clock. `spawn_blocking` keeps
/// the work off the main thread regardless — today that's just a trivial
/// JSON parse, but a real implementation will do real filesystem I/O
/// here.
#[command]
pub async fn library() -> Result<Library, String> {
    tauri::async_runtime::spawn_blocking(demo_library)
        .await
        .map_err(|e| e.to_string())
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
/// API, no engine dependency. Rebuilds the window if it doesn't exist
/// (review round 1, blocking #5): closing main only ever hides it now
/// (see `tray::setup`), but this stays defensive in case some other path
/// destroys it — the alternative is a hidden, unreachable process the
/// owner can only kill from Activity Monitor / Task Manager.
#[command]
pub fn show_main_window(app: AppHandle) -> Result<(), String> {
    let window = ensure_main_window(&app).map_err(|e| e.to_string())?;
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}
