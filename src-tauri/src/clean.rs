//! Removing what a scan found. Only items from the last scan's report can be removed, and
//! each one is re-checked right before it is touched.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use serde::Serialize;

use crate::fsutil;
use crate::inventory::Inventory;
use crate::report::{Report, Tier};
use crate::roots::Roots;
use crate::rules::Rules;
use crate::scan::Ctx;

/// What this OS calls the place deleted things go. Used in the Rust messages below and handed
/// to the page through `AppInfo`, so there is one spelling of it in the whole app.
#[cfg(windows)]
pub const BIN_NAME: &str = "Recycle Bin";
#[cfg(not(windows))]
pub const BIN_NAME: &str = "Trash";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    RecycleBin,
    Permanent,
    /// Not attempted: a safety check said no.
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    pub finding_id: String,
    pub title: String,
    pub path: String,
    pub bytes: u64,
    pub method: Method,
    /// Why it was skipped or what went wrong. None means it fully worked.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CleanResult {
    pub outcomes: Vec<Outcome>,
    /// Space freed right away by permanent deletes.
    pub freed_bytes: u64,
    /// Space moved to the Recycle Bin (freed once it's emptied).
    pub recycled_bytes: u64,
    pub skipped: usize,
    pub started_at: u64,
    /// Where to reveal in the file manager so the user can put things back by hand.
    ///
    /// Set only where in-app undo is unavailable, which is how the page decides between an
    /// "Undo" button and a "Show in Trash" one — no platform check needed in the page itself.
    pub reveal_dir: Option<String>,
}

impl CleanResult {
    pub fn recycled_paths(&self) -> Vec<String> {
        self.outcomes
            .iter()
            .filter(|o| o.method == Method::RecycleBin && o.error.is_none())
            .map(|o| o.path.clone())
            .collect()
    }
}

/// How items are moved to the Recycle Bin / Trash.
///
/// On macOS the `trash` crate defaults to `DeleteMethod::Finder`, which runs
/// `osascript -e 'tell application "Finder" to delete ...'`. That is a subprocess and an Apple
/// Events consent prompt, and without an `NSAppleEventsUsageDescription` string macOS kills the
/// process outright rather than returning an error. `NsFileManager` calls `trashItemAtURL:`
/// directly: no subprocess, no extra permission, and faster. The cost is that Finder's "Put
/// Back" may not appear, so recovery is dragging the item out of the Trash.
fn trash_ctx() -> trash::TrashContext {
    #[allow(unused_mut)]
    let mut ctx = trash::TrashContext::default();
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        ctx.set_delete_method(DeleteMethod::NsFileManager);
    }
    ctx
}

/// The checks used right before removing: protected paths and what is running now.
pub fn guard_ctx() -> Ctx {
    Ctx::new(Roots::detect(), Rules::load(), Inventory::running_only())
}

/// Remove the items of the given findings. Safe-tier items are deleted permanently when
/// `permanent_safe` is set; everything else goes to the Recycle Bin.
pub fn clean(report: &Report, finding_ids: &[String], permanent_safe: bool, ctx: &Ctx) -> CleanResult {
    let mut result = CleanResult { started_at: fsutil::now(), ..CleanResult::default() };
    let bin = trash_ctx();
    for id in finding_ids {
        let Some(finding) = report.findings.iter().find(|f| &f.id == id) else {
            result.outcomes.push(Outcome {
                finding_id: id.clone(),
                title: String::new(),
                path: String::new(),
                bytes: 0,
                method: Method::Skipped,
                error: Some("Not in the last scan. Scan again and retry.".into()),
            });
            result.skipped += 1;
            continue;
        };
        let method = if finding.tier == Tier::Safe && permanent_safe { Method::Permanent } else { Method::RecycleBin };
        for item in &finding.items {
            let path = Path::new(&item.path);
            let mut outcome = Outcome {
                finding_id: finding.id.clone(),
                title: finding.title.clone(),
                path: item.path.clone(),
                bytes: item.bytes,
                method,
                error: None,
            };
            if let Some(reason) = refuse_reason(path, ctx, finding.vouched) {
                outcome.method = Method::Skipped;
                outcome.error = Some(reason.into());
            } else {
                match method {
                    Method::RecycleBin => match bin.delete(path) {
                        Ok(()) => result.recycled_bytes += item.bytes,
                        Err(e) => outcome.error = Some(format!("Couldn't move it to the {BIN_NAME}: {e}")),
                    },
                    Method::Permanent => {
                        let failures = remove_tree(path);
                        let left = if path.exists() { fsutil::usage(path).bytes } else { 0 };
                        result.freed_bytes += item.bytes.saturating_sub(left);
                        if failures > 0 {
                            outcome.error = Some(format!("{failures} files or folders were in use and were left behind."));
                        }
                    }
                    Method::Skipped => unreachable!(),
                }
            }
            if outcome.method == Method::Skipped {
                result.skipped += 1;
            }
            result.outcomes.push(outcome);
        }
    }
    // macOS has no API to list or restore the Trash, so there is nothing to undo through.
    // Point the user at it instead — Finder can put things back by hand.
    //
    // Counted, not measured: empty folders and 0-byte stray files are real removals worth
    // offering a way back from, and gating on bytes would leave the page showing an Undo button
    // that this platform cannot honour.
    if !cfg!(windows) && !result.recycled_paths().is_empty() {
        result.reveal_dir = trash_reveal_target(ctx);
    }
    result
}

/// What to hand the page's "Show in Trash" action.
///
/// Revealing `~/.Trash` itself is wrong: the reveal shows an item *in its parent*, so Finder
/// would open the home folder with an invisible `.Trash` entry selected. Naming the newest thing
/// inside instead opens the Trash window with the just-removed item highlighted, which is what
/// the button says it does.
///
/// `trash::delete` returns `()` and never says what the item was renamed to on a collision
/// (`foo` may land as `foo 2`), so "newest" is the best available handle on it.
/// Known limitation: this assumes the home Trash. Something removed from a non-boot volume goes
/// to that volume's `.Trashes/<uid>` instead. Everything ChillSweep scans lives under `$HOME`,
/// so that case does not arise today — but if the scan ever reaches another volume, this would
/// point at the wrong folder and needs deriving from the removed path instead.
#[cfg(not(windows))]
fn trash_reveal_target(ctx: &Ctx) -> Option<String> {
    let trash = ctx.roots.home.join(".Trash");
    if !trash.is_dir() {
        return None;
    }
    let newest = fsutil::children(&trash).into_iter().max_by_key(|(_, md)| fsutil::mtime(md)).map(|(p, _)| p);
    newest.unwrap_or(trash).to_str().map(str::to_string)
}

#[cfg(windows)]
fn trash_reveal_target(_ctx: &Ctx) -> Option<String> {
    None
}

fn refuse_reason(path: &Path, ctx: &Ctx, vouched: bool) -> Option<&'static str> {
    if let Some(reason) = ctx.refuse_location(path, vouched) {
        Some(reason)
    } else if ctx.inv.running_inside(path).is_some() {
        Some("A running program is using it. Close the program and try again.")
    } else if fs::symlink_metadata(path).is_err() {
        Some("It no longer exists.")
    } else {
        None
    }
}

/// Delete a file or folder tree, carrying on past anything locked. Never follows
/// junctions or symlinks: a link itself is removed, not what it points at.
/// Returns how many entries couldn't be removed.
fn remove_tree(path: &Path) -> u64 {
    let Ok(md) = fs::symlink_metadata(path) else { return 0 };
    let mut failures = 0;
    if md.is_dir() {
        if !fsutil::is_reparse(&md) {
            if let Ok(rd) = fs::read_dir(path) {
                for entry in rd.flatten() {
                    failures += remove_tree(&entry.path());
                }
            }
        }
        if fs::remove_dir(path).is_err() && failures == 0 {
            failures += 1;
        }
    } else if fs::remove_file(path).is_err() {
        let mut perms = md.permissions();
        if perms.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            let _ = fs::set_permissions(path, perms);
        }
        if fs::remove_file(path).is_err() {
            failures += 1;
        }
    }
    failures
}

/// Append one JSON line per outcome to `<dir>\history.jsonl`.
pub fn append_history(dir: &Path, result: &CleanResult) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let mut file = OpenOptions::new().create(true).append(true).open(dir.join("history.jsonl"))?;
    for o in &result.outcomes {
        let line = serde_json::json!({
            "time": result.started_at,
            "path": o.path,
            "bytes": o.bytes,
            "method": o.method,
            "error": o.error,
        });
        writeln!(file, "{line}")?;
    }
    Ok(())
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RestoreFailure {
    pub path: String,
    pub error: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RestoreResult {
    pub restored: Vec<String>,
    pub failed: Vec<RestoreFailure>,
}

/// Put items back from the Recycle Bin: the newest entry for each original path that
/// was deleted at or after `since`.
#[cfg(windows)]
pub fn undo(paths: &[String], since: u64) -> RestoreResult {
    let mut result = RestoreResult::default();
    let bin = match trash::os_limited::list() {
        Ok(items) => items,
        Err(e) => {
            result.failed = paths.iter().map(|p| RestoreFailure { path: p.clone(), error: format!("Couldn't read the {BIN_NAME}: {e}") }).collect();
            return result;
        }
    };
    // Recycle Bin timestamps have coarse resolution; allow a little slack.
    let since = since as i64 - 5;
    for path in paths {
        let wanted = path.to_lowercase();
        let newest = bin
            .iter()
            .filter(|i| i.time_deleted >= since && i.original_path().to_string_lossy().to_lowercase() == wanted)
            .max_by_key(|i| i.time_deleted);
        match newest {
            None => result.failed.push(RestoreFailure { path: path.clone(), error: format!("It's no longer in the {BIN_NAME}.") }),
            Some(item) => match trash::os_limited::restore_all([item.clone()]) {
                Ok(()) => result.restored.push(path.clone()),
                Err(trash::Error::RestoreCollision { .. }) => result.failed.push(RestoreFailure {
                    path: path.clone(),
                    error: format!("Something new already exists at that location, so it was left in the {BIN_NAME}."),
                }),
                Err(e) => result.failed.push(RestoreFailure { path: path.clone(), error: e.to_string() }),
            },
        }
    }
    result
}

/// macOS gives no way to read or restore the Trash — `trash::os_limited` is Windows and
/// Freedesktop only — so there is nothing to undo through. The UI offers "Show in Trash"
/// instead, and Finder's own Put Back does the job. Nothing is lost either way: the items are
/// still in the Trash.
#[cfg(not(windows))]
pub fn undo(paths: &[String], _since: u64) -> RestoreResult {
    RestoreResult {
        restored: Vec::new(),
        failed: paths
            .iter()
            .map(|p| RestoreFailure {
                path: p.clone(),
                error: "Undo isn't available on this platform. Open the Trash and use Put Back.".into(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Category, Confidence, Finding, InventorySummary, Item};

    fn report_with(tier: Tier, path: &Path) -> Report {
        let u = fsutil::usage(path);
        Report {
            findings: vec![Finding {
                id: "f1".into(),
                title: "Test".into(),
                tier,
                category: Category::Cache,
                confidence: Confidence::High,
                what: None,
                if_deleted: None,
                evidence: vec![],
                items: vec![Item { path: path.to_string_lossy().into_owned(), bytes: u.bytes, files: u.files, is_dir: u.is_dir }],
                vouched: false,
                bytes: u.bytes,
                last_modified: None,
            }],
            totals: vec![],
            projects: Default::default(),
            inventory: InventorySummary::default(),
            duration_ms: 0,
            warnings: vec![],
        }
    }

    fn ctx() -> Ctx {
        Ctx::new(Roots::detect(), Rules::load(), Inventory::default())
    }

    fn fixture(root: &Path) -> std::path::PathBuf {
        let dir = root.join("chillsweep-test-fixture");
        fs::create_dir_all(dir.join("nested")).unwrap();
        fs::write(dir.join("a.bin"), vec![0u8; 2048]).unwrap();
        fs::write(dir.join("nested").join("b.bin"), vec![0u8; 1024]).unwrap();
        dir
    }

    #[test]
    fn rejects_ids_not_in_report() {
        let report = Report { findings: vec![], totals: vec![], projects: Default::default(), inventory: InventorySummary::default(), duration_ms: 0, warnings: vec![] };
        let r = clean(&report, &["nope".into()], true, &ctx());
        assert_eq!(r.skipped, 1);
        assert_eq!(r.outcomes[0].method, Method::Skipped);
    }

    #[test]
    fn rejects_protected_paths() {
        // Inside ~/.ssh, and deliberately nonexistent so a bug could never touch real files.
        let path = Roots::detect().home.join(".ssh").join("chillsweep-test-does-not-exist");
        let r = clean(&report_with(Tier::Leftover, &path), &["f1".into()], true, &ctx());
        assert_eq!(r.outcomes[0].method, Method::Skipped);
        // `.ssh` is on the absolute list, so it reports the stronger of the two refusals.
        assert_eq!(r.outcomes[0].error.as_deref(), Some("This location is off limits."));
    }

    /// The whole point of vouching: a signal that can prove what something is may act inside a
    /// merely-protected folder, but the absolute list is still absolute.
    #[test]
    fn vouching_passes_protected_but_never_the_absolute_list() {
        let ctx = ctx();
        let home = Roots::detect().home;
        let in_documents = home.join("Documents").join("chillsweep-test-does-not-exist");
        let in_ssh = home.join(".ssh").join("chillsweep-test-does-not-exist");

        assert!(ctx.refuse_location(&in_documents, false).is_some(), "Documents is protected");
        assert!(ctx.refuse_location(&in_documents, true).is_none(), "proof should get past protected");
        assert_eq!(ctx.refuse_location(&in_ssh, true), Some("This location is off limits."), "nothing gets past this");
    }

    #[test]
    fn permanent_delete_removes_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = fixture(tmp.path());
        let r = clean(&report_with(Tier::Safe, &dir), &["f1".into()], true, &ctx());
        assert_eq!(r.outcomes[0].method, Method::Permanent);
        assert!(r.outcomes[0].error.is_none());
        assert_eq!(r.freed_bytes, 3072);
        assert!(!dir.exists());
    }

    // Mandatory locking is a Windows thing: on macOS an open file deletes happily.
    #[cfg(windows)]
    #[test]
    fn locked_file_is_left_and_reported() {
        use std::os::windows::fs::OpenOptionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = fixture(tmp.path());
        let _lock = OpenOptions::new().read(true).share_mode(0).open(dir.join("a.bin")).unwrap();
        let r = clean(&report_with(Tier::Safe, &dir), &["f1".into()], true, &ctx());
        assert!(r.outcomes[0].error.as_deref().unwrap().contains("in use"));
        assert!(dir.join("a.bin").exists());
        assert!(!dir.join("nested").exists());
        assert_eq!(r.freed_bytes, 1024);
    }

    #[cfg(windows)]
    #[test]
    fn recycle_then_undo_restores() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = fixture(tmp.path());
        let r = clean(&report_with(Tier::Leftover, &dir), &["f1".into()], true, &ctx());
        assert_eq!(r.outcomes[0].method, Method::RecycleBin, "{:?}", r.outcomes[0].error);
        assert!(r.outcomes[0].error.is_none(), "{:?}", r.outcomes[0].error);
        assert!(!dir.exists());
        let restored = undo(&r.recycled_paths(), r.started_at);
        assert!(restored.failed.is_empty(), "{:?}", restored.failed);
        assert!(dir.join("nested").join("b.bin").exists());
    }

    #[test]
    fn writes_history() {
        let tmp = tempfile::tempdir().unwrap();
        let report = Report { findings: vec![], totals: vec![], projects: Default::default(), inventory: InventorySummary::default(), duration_ms: 0, warnings: vec![] };
        let r = clean(&report, &["x".into()], false, &ctx());
        append_history(tmp.path(), &r).unwrap();
        let text = fs::read_to_string(tmp.path().join("history.jsonl")).unwrap();
        assert!(text.contains("\"method\":\"skipped\""));
    }
}
