pub mod clean;
pub mod fsutil;
pub mod inventory;
pub mod progress;
pub mod report;
pub mod roots;
pub mod rules;
pub mod scan;
mod signals;
mod titlebar;

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tauri::window::{ProgressBarState, ProgressBarStatus};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

use crate::progress::{Reporter, ScanProgress};
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

/// How often the dock or taskbar bar may be redrawn.
///
/// Far less often than the page: on Windows each call builds a COM taskbar object and on macOS it
/// forces a redraw of the dock icon, while four a second is already more than anyone reads from an
/// icon the size of a thumbnail.
const OS_PROGRESS_MS: u64 = 250;

/// The progress bar on the dock icon (macOS) or the taskbar button (Windows).
///
/// Tauri drives both from one call, so there is no platform code here. Clearing it is done on
/// `Drop` so that it cannot be left behind by an early return or an error.
struct OsProgress {
    window: Option<WebviewWindow>,
    base: Instant,
    last_ms: AtomicU64,
    /// Last whole percent sent, so repeats of the same number cost nothing.
    last_percent: AtomicI64,
}

impl OsProgress {
    fn new(app: &AppHandle) -> OsProgress {
        OsProgress {
            window: app.get_webview_window("main"),
            base: Instant::now(),
            last_ms: AtomicU64::new(0),
            last_percent: AtomicI64::new(-1),
        }
    }

    fn set(&self, percent: f32) {
        let Some(window) = self.window.as_ref() else { return };
        let whole = percent.clamp(0.0, 100.0).round() as i64;
        if self.last_percent.load(Ordering::Relaxed) == whole {
            return;
        }
        let now = self.base.elapsed().as_millis() as u64;
        let last = self.last_ms.load(Ordering::Relaxed);
        if now.saturating_sub(last) < OS_PROGRESS_MS {
            return;
        }
        if self.last_ms.compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed).is_err() {
            return;
        }
        self.last_percent.store(whole, Ordering::Relaxed);
        let _ = window.set_progress_bar(ProgressBarState {
            status: Some(ProgressBarStatus::Normal),
            progress: Some(whole as u64),
        });
    }
}

impl Drop for OsProgress {
    fn drop(&mut self) {
        if let Some(window) = self.window.as_ref() {
            let _ = window.set_progress_bar(ProgressBarState { status: Some(ProgressBarStatus::None), progress: None });
        }
    }
}

/// Scan the user's folders. Read-only: nothing is changed, nothing else is launched.
#[tauri::command]
async fn scan(app: AppHandle, state: State<'_, AppState>) -> Result<report::Report, String> {
    let report = tauri::async_runtime::spawn_blocking(move || {
        let os = Arc::new(OsProgress::new(&app));
        let sink_app = app.clone();
        let sink_os = Arc::clone(&os);
        let reporter = Reporter::new(
            progress::scan_stages(),
            Arc::new(move |p: &ScanProgress| {
                let _ = sink_app.emit("scan-progress", p);
                sink_os.set(p.percent);
            }),
        );
        scan::run_with_progress(reporter)
        // `os` drops here, which clears the dock bar however the scan ended.
    })
    .await
    .map_err(|e| e.to_string())?;
    *state.last_report.lock().unwrap() = Some(report.clone());
    Ok(report)
}

/// A clean, and the report brought up to date by it.
///
/// The page renders from `report` instead of scanning again: the findings it holds are
/// non-overlapping, so removing some of them cannot change the rest. See `clean::reconcile`.
#[derive(serde::Serialize)]
struct CleanResponse {
    result: clean::CleanResult,
    report: report::Report,
}

/// Remove the chosen findings from the last scan.
#[tauri::command]
async fn clean(
    app: AppHandle,
    state: State<'_, AppState>,
    finding_ids: Vec<String>,
    permanent_safe: bool,
) -> Result<CleanResponse, String> {
    // Taking the report means nothing else can clean against it while this one runs. The version
    // brought up to date by the clean goes back below, so the next clean needs no fresh scan.
    let mut report = state.last_report.lock().unwrap().take().ok_or("Scan first, then choose what to clean.")?;
    let (result, report) = tauri::async_runtime::spawn_blocking(move || {
        let os = OsProgress::new(&app);
        let ctx = clean::guard_ctx();
        let result = clean::clean(&report, &finding_ids, permanent_safe, &ctx, &|p| {
            let _ = app.emit("clean-progress", p);
            os.set(p.done as f32 / p.total.max(1) as f32 * 100.0);
        });
        let _ = clean::append_history(&ctx.roots.app_data_dir(), &result);
        clean::reconcile(&mut report, &result.outcomes);
        (result, report)
    })
    .await
    .map_err(|e| e.to_string())?;
    *state.last_recycled.lock().unwrap() = Some((result.recycled_paths(), result.started_at));
    *state.last_report.lock().unwrap() = Some(report.clone());
    Ok(CleanResponse { result, report })
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
