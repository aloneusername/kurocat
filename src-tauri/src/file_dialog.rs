//! System file pickers opened from the context menu.
//!
//! Pickers run on the main thread with rfd's blocking API: rfd's async API
//! attaches the picker as a sheet to the pet window on macOS, which makes the
//! system move the pet out of the way. The menu is hidden first and we wait for
//! that to happen, because a hide request still queued behind the picker's
//! modal loop would leave the always-on-top menu covering the picker.

use std::path::PathBuf;

use tauri::AppHandle;

fn hide_menu_now(app: &AppHandle) {
    use std::sync::atomic::Ordering;
    use tauri::Manager;
    if let Some(w) = app.get_webview_window("menu") {
        let (tx, rx) = std::sync::mpsc::channel();
        if app
            .run_on_main_thread(move || {
                let _ = w.hide();
                let _ = tx.send(());
            })
            .is_ok()
        {
            let _ = rx.recv_timeout(std::time::Duration::from_millis(500));
        }
    }
    app.state::<crate::AppState>()
        .menu_open
        .store(false, Ordering::Relaxed);
}

async fn run_dialog<F>(app: &AppHandle, show: F) -> Result<Option<PathBuf>, String>
where
    F: FnOnce() -> Option<PathBuf> + Send + 'static,
{
    hide_menu_now(app);
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(show());
    })
    .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || rx.recv())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Lets the user choose one existing file. `None` when cancelled.
pub async fn pick_file(
    app: &AppHandle,
    title: &'static str,
    filter_name: &'static str,
    extensions: &'static [&'static str],
) -> Result<Option<PathBuf>, String> {
    run_dialog(app, move || {
        rfd::FileDialog::new()
            .set_title(title)
            .add_filter(filter_name, extensions)
            .pick_file()
    })
    .await
}

/// Lets the user choose where to save a file. `None` when cancelled.
pub async fn save_file(
    app: &AppHandle,
    title: &'static str,
    default_name: String,
    filter_name: &'static str,
    extensions: &'static [&'static str],
) -> Result<Option<PathBuf>, String> {
    run_dialog(app, move || {
        rfd::FileDialog::new()
            .set_title(title)
            .set_file_name(default_name)
            .add_filter(filter_name, extensions)
            .save_file()
    })
    .await
}
