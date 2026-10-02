pub mod clean;
pub mod fsutil;
pub mod inventory;
pub mod report;
pub mod roots;
pub mod rules;
pub mod scan;
mod signals;
mod titlebar;

use std::sync::Mutex;

use tauri::{AppHandle, Manager, State};
use tauri_plugin_updater::UpdaterExt;

#[derive(Default)]
struct AppState {
    /// The report the UI is showing. Only its findings can be removed.
    last_report: Mutex<Option<report::Report>>,
    /// What the last clean sent to the Recycle Bin, and when, for undo.
    last_recycled: Mutex<Option<(Vec<String>, u64)>>,
    /// An update found by `check_update`, waiting for the user to install it.
    pending_update: Mutex<Option<tauri_plugin_updater::Update>>,
}

/// Scan the user's folders. Read-only: nothing is changed, nothing else is launched.
#[tauri::command]
async fn scan(state: State<'_, AppState>) -> Result<report::Report, String> {
    let report = tauri::async_runtime::spawn_blocking(scan::run).await.map_err(|e| e.to_string())?;
    *state.last_report.lock().unwrap() = Some(report.clone());
    Ok(report)
}

/// Remove the chosen findings from the last scan.
#[tauri::command]
async fn clean(state: State<'_, AppState>, finding_ids: Vec<String>, permanent_safe: bool) -> Result<clean::CleanResult, String> {
    // Taking the report means a fresh scan is needed before cleaning again.
    let report = state.last_report.lock().unwrap().take().ok_or("Scan first, then choose what to clean.")?;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let ctx = clean::guard_ctx();
        let result = clean::clean(&report, &finding_ids, permanent_safe, &ctx);
        let _ = clean::append_history(&ctx.roots.app_data_dir(), &result);
        result
    })
    .await
    .map_err(|e| e.to_string())?;
    *state.last_recycled.lock().unwrap() = Some((result.recycled_paths(), result.started_at));
    Ok(result)
}

/// Restore what the last clean sent to the Recycle Bin.
#[tauri::command]
async fn undo_last(state: State<'_, AppState>) -> Result<clean::RestoreResult, String> {
    let (paths, since) = state.last_recycled.lock().unwrap().take().ok_or("Nothing to undo.")?;
    tauri::async_runtime::spawn_blocking(move || clean::undo(&paths, since)).await.map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
struct AppInfo {
    version: String,
    git_hash: &'static str,
    /// Unix seconds.
    build_time: u64,
    /// `"windows"` or `"macos"`. The page uses it to pick wording, so it never has to guess
    /// from the user agent.
    platform: &'static str,
    /// What this OS calls the place deleted things go: "Recycle Bin" or "Trash".
    bin_name: &'static str,
    /// What this OS calls its file manager: "Explorer" or "Finder".
    file_manager: &'static str,
}

/// Version and build details for the About screen.
#[tauri::command]
fn app_info(app: AppHandle) -> AppInfo {
    AppInfo {
        // Set per build by build-scripts/pack.mjs (1.{commit count}.0), so it matches what the updater compares.
        version: app.package_info().version.to_string(),
        git_hash: env!("CHILLSWEEP_GIT_HASH"),
        build_time: env!("CHILLSWEEP_BUILD_TIME").parse().unwrap_or(0),
        platform: if cfg!(windows) { "windows" } else { "macos" },
        bin_name: clean::BIN_NAME,
        file_manager: if cfg!(windows) { "Explorer" } else { "Finder" },
    }
}

#[derive(serde::Serialize)]
struct UpdateInfo {
    version: String,
}

/// Ask the releases feed whether a newer version is published. Drafts never appear there.
/// Debug builds never check, so `tauri dev` is never offered an update.
#[tauri::command]
async fn check_update(app: AppHandle, state: State<'_, AppState>) -> Result<Option<UpdateInfo>, String> {
    if cfg!(debug_assertions) {
        return Ok(None);
    }
    let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    let info = update.as_ref().map(|u| UpdateInfo { version: u.version.clone() });
    *state.pending_update.lock().unwrap() = update;
    Ok(info)
}

/// Download, verify and run the installer for the update `check_update` found. On Windows the
/// installer closes the app and starts the new version itself, so this only returns on failure.
#[tauri::command]
async fn install_update(state: State<'_, AppState>) -> Result<(), String> {
    let update = state.pending_update.lock().unwrap().take().ok_or("No update to install.")?;
    update.download_and_install(|_, _| {}, || {}).await.map_err(|e| e.to_string())
}

/// Recolour the title bar when the page switches between light and dark.
#[tauri::command]
fn set_titlebar_theme(window: tauri::WebviewWindow, dark: bool) {
    titlebar::style(&window, dark);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState::default())
        .setup(|app| {
            // Dark is the default theme; the page corrects it on load if light is chosen.
            if let Some(window) = app.get_webview_window("main") {
                titlebar::style(&window, true);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![scan, clean, undo_last, app_info, check_update, install_update, set_titlebar_theme])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
