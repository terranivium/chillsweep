//! Read-only filesystem helpers. Nothing in here ever writes, and nothing follows
//! junctions or symlinks (Windows keeps compatibility junctions like
//! `Application Data` that loop back on themselves).

use std::fs::{self, Metadata};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

pub const DAY: u64 = 86_400;

pub fn is_reparse(md: &Metadata) -> bool {
    md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

pub fn is_hidden_system(md: &Metadata) -> bool {
    let a = md.file_attributes();
    a & FILE_ATTRIBUTE_HIDDEN != 0 && a & FILE_ATTRIBUTE_SYSTEM != 0
}

pub fn mtime(md: &Metadata) -> u64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Usage {
    pub bytes: u64,
    pub files: u64,
    /// Newest mtime of any file or folder inside (including the root).
    pub newest: u64,
    pub is_dir: bool,
}

/// Total size, file count and newest modification time of a file or folder tree.
pub fn usage(path: &Path) -> Usage {
    let Ok(md) = fs::symlink_metadata(path) else {
        return Usage::default();
    };
    if is_reparse(&md) {
        return Usage { newest: mtime(&md), is_dir: md.is_dir(), ..Usage::default() };
    }
    if !md.is_dir() {
        return Usage { bytes: md.len(), files: 1, newest: mtime(&md), is_dir: false };
    }
    let mut u = Usage { newest: mtime(&md), is_dir: true, ..Usage::default() };
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            // On Windows DirEntry::metadata does not follow reparse points.
            let Ok(md) = entry.metadata() else { continue };
            if is_reparse(&md) {
                continue;
            }
            u.newest = u.newest.max(mtime(&md));
            if md.is_dir() {
                stack.push(entry.path());
            } else {
                u.bytes += md.len();
                u.files += 1;
            }
        }
    }
    u
}

/// Children of a directory with their metadata, skipping reparse points.
pub fn children(dir: &Path) -> Vec<(PathBuf, Metadata)> {
    let Ok(rd) = fs::read_dir(dir) else { return Vec::new() };
    rd.flatten()
        .filter_map(|e| {
            let md = e.metadata().ok()?;
            (!is_reparse(&md)).then(|| (e.path(), md))
        })
        .collect()
}

pub fn child_dirs(dir: &Path) -> Vec<PathBuf> {
    children(dir).into_iter().filter(|(_, md)| md.is_dir()).map(|(p, _)| p).collect()
}

/// Visit directories breadth-first down to `max_depth` (root is depth 0). `visit` returns
/// false to stop descending into that directory.
pub fn walk_dirs(root: &Path, max_depth: usize, mut visit: impl FnMut(&Path, usize) -> bool) {
    let mut queue = std::collections::VecDeque::from([(root.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = queue.pop_front() {
        if !visit(&dir, depth) || depth >= max_depth {
            continue;
        }
        for child in child_dirs(&dir) {
            queue.push_back((child, depth + 1));
        }
    }
}

pub fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn lower(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

/// True if `inner` is `outer` or somewhere below it (case-insensitive).
pub fn is_within(inner: &str, outer: &str) -> bool {
    let inner = inner.trim_end_matches('\\');
    let outer = outer.trim_end_matches('\\');
    inner == outer || (inner.len() > outer.len() && inner.starts_with(outer) && inner.as_bytes()[outer.len()] == b'\\')
}

/// "today", "3 days ago", or "on 2023-10-18".
pub fn when(now: u64, t: u64) -> String {
    if t == 0 {
        return "at an unknown time".into();
    }
    let days = now.saturating_sub(t) / DAY;
    match days {
        0 => "today".into(),
        1 => "yesterday".into(),
        2..=59 => format!("{days} days ago"),
        _ => format!("on {}", date(t)),
    }
}

/// Unix seconds to YYYY-MM-DD (UTC), without pulling in a date crate.
pub fn date(t: u64) -> String {
    let z = (t / DAY) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Case-insensitive match with `*` as the only wildcard.
pub fn wildcard(pattern: &str, text: &str) -> bool {
    let p = pattern.to_lowercase();
    let t = text.to_lowercase();
    let parts: Vec<&str> = p.split('*').collect();
    if parts.len() == 1 {
        return p == t;
    }
    let mut rest = t.as_str();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            let Some(r) = rest.strip_prefix(part) else { return false };
            rest = r;
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else if let Some(pos) = rest.find(part) {
            rest = &rest[pos + part.len()..];
        } else {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_790_000_000), "2026-09-21");
    }

    #[test]
    fn wildcards() {
        assert!(wildcard("*-updater", "obsidian-updater"));
        assert!(wildcard("*-updater", "CurseForge-Updater"));
        assert!(!wildcard("*-updater", "updater-x"));
        assert!(wildcard("a*c*e", "abcde"));
        assert!(wildcard("cache", "Cache"));
    }

    #[test]
    fn within() {
        assert!(is_within(r"c:\a\b", r"c:\a"));
        assert!(is_within(r"c:\a", r"c:\a"));
        assert!(!is_within(r"c:\ab", r"c:\a"));
    }
}
