//! Older copies of an app kept next to the current one: version-numbered siblings
//! (`2.1.181`, `2.1.283`, `app-1.0.9`) and files renamed to `*.old*` during self-updates.

use std::path::{Path, PathBuf};

use regex::Regex;

use super::{finding, item, last_changed, Taken};
use crate::fsutil;
use crate::report::{Category, Confidence, Finding, Tier};
use crate::scan::Ctx;

/// Folder names that just group versions; the title uses the parent's name instead.
const GROUPING_NAMES: [&str; 6] = ["versions", "bin", "app", "current", "releases", "share"];

/// Program files renamed during a self-update (`claude.exe.old.123`, `Foo.app.old`), as opposed
/// to a database's rotated logs (`LOG.old`). The extension list is what makes it a *program*
/// file rather than any old backup, so it differs per platform.
#[cfg(windows)]
const OLD_FILE_RE: &str = r"(?i)\.(?:exe|dll|msi|asar|zip)\.old(?:\.\d+)?$";
#[cfg(not(windows))]
const OLD_FILE_RE: &str = r"(?i)\.(?:app|dylib|framework|so|asar|zip|pkg)\.old(?:\.\d+)?$";
const MIN_BYTES: u64 = 10 << 20;

pub fn find(ctx: &Ctx, taken: &Taken) -> Vec<Finding> {
    let version_re = Regex::new(r"^(?:app-|v)?(\d+(?:\.\d+){1,3})$").unwrap();
    // Program files renamed during a self-update (`claude.exe.old.123`), not databases'
    // rotated logs (`LOG.old`).
    let old_re = Regex::new(OLD_FILE_RE).unwrap();
    let mut out = Vec::new();
    for (root, depth) in ctx.roots.version_roots() {
        fsutil::walk_dirs(&root, depth, |dir, d| {
            if d == 1 && (ctx.rules.is_system_name(&fsutil::file_name(dir)) || !ctx.may_remove(dir, false)) {
                return false;
            }
            if taken.covers(dir) {
                return false;
            }
            if let Some(f) = check_dir(ctx, dir, &version_re, &old_re) {
                out.push(f);
            }
            true
        });
    }
    out
}

fn check_dir(ctx: &Ctx, dir: &Path, version_re: &Regex, old_re: &Regex) -> Option<Finding> {
    let entries = fsutil::children(dir);
    let mut versions: Vec<(Vec<u64>, PathBuf)> = entries
        .iter()
        .filter_map(|(p, _)| {
            let name = fsutil::file_name(p);
            let caps = version_re.captures(&name)?;
            let v = caps[1].split('.').map(|n| n.parse().unwrap_or(0)).collect();
            Some((v, p.clone()))
        })
        .collect();
    let old_files: Vec<PathBuf> = entries
        .iter()
        .filter(|(p, md)| md.is_file() && old_re.is_match(&fsutil::file_name(p)))
        .map(|(p, _)| p.clone())
        .collect();
    if versions.len() < 2 && old_files.is_empty() {
        return None;
    }

    let mut evidence = Vec::new();
    let mut targets = Vec::new();
    if versions.len() >= 2 {
        versions.sort();
        let (newest, _) = versions.pop().unwrap();
        let older: Vec<_> = versions
            .into_iter()
            .filter(|(_, p)| ctx.inv.running_inside(p).is_none() && ctx.inv.referenced_inside(p).is_none())
            .collect();
        if !older.is_empty() {
            let names: Vec<_> = older.iter().map(|(_, p)| fsutil::file_name(p)).collect();
            evidence.push(format!("The newest version here is {}; these are older: {}.", join_version(&newest), names.join(", ")));
            targets.extend(older.into_iter().map(|(_, p)| p));
        }
    }
    if !old_files.is_empty() {
        let names: Vec<_> = old_files.iter().map(|p| fsutil::file_name(p)).collect();
        evidence.push(format!("{} renamed to “.old” when the app updated itself.", names.join(", ")));
        targets.extend(old_files);
    }
    if targets.is_empty() {
        return None;
    }

    let app = app_name(dir);
    let mut f = finding(format!("versions:{}", fsutil::lower(dir)), format!("Old {app} versions"), Tier::Safe, Category::Leftover, Confidence::Medium);
    f.what = Some(format!("Earlier copies of {app} kept after it updated."));
    f.if_deleted = Some("Nothing, unless you want to roll back to an older version.".into());
    let mut newest = 0;
    for t in &targets {
        let u = fsutil::usage(t);
        newest = newest.max(u.newest);
        f.items.push(item(t, &u));
    }
    f.recompute_bytes();
    // Tiny version-named folders are usually per-version settings (e.g. one per Unreal
    // Engine release), not old copies of an app.
    if f.bytes < MIN_BYTES {
        return None;
    }
    f.evidence = evidence;
    f.evidence.push(last_changed(ctx, newest));
    f.last_modified = Some(newest);
    Some(f)
}

fn join_version(v: &[u64]) -> String {
    v.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(".")
}

fn app_name(dir: &Path) -> String {
    dir.ancestors()
        .map(fsutil::file_name)
        .find(|n| !n.is_empty() && !GROUPING_NAMES.contains(&n.to_lowercase().as_str()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::app_name;

    /// `app_name` walks up parent folders, so the test only needs a real path shape.
    fn at(parts: &[&str]) -> std::path::PathBuf {
        parts.iter().fold(std::path::PathBuf::from(std::path::MAIN_SEPARATOR_STR), |acc, p| acc.join(p))
    }

    #[test]
    fn app_names_skip_grouping_folders() {
        assert_eq!(app_name(&at(&["home", "x", ".local", "share", "claude", "versions"])), "claude");
        assert_eq!(app_name(&at(&["home", "x", "AppData", "Local", "Discord"])), "Discord");
    }
}
