pub mod clean;
pub mod fsutil;
pub mod inventory;
pub mod report;
pub mod roots;
pub mod rules;
pub mod scan;
mod signals;

use std::sync::Mutex;

use tauri::State;

#[derive(Default)]
struct AppState {
    /// The report the UI is showing. Only its findings can be removed.
    last_report: Mutex<Option<report::Report>>,
    /// What the last clean sent to the Recycle Bin, and when, for undo.
    last_recycled: Mutex<Option<(Vec<String>, u64)>>,
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
        let _ = clean::append_history(&ctx.roots.local.join("Leftover"), &result);
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![scan, clean, undo_last])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
