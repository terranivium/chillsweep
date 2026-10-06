//! Read-only filesystem helpers. Nothing in here ever writes, and nothing follows
//! junctions or symlinks (Windows keeps compatibility junctions like
//! `Application Data` that loop back on themselves).

use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DAY: u64 = 86_400;

#[cfg(windows)]
mod attrs {
    use std::fs::Metadata;
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

    /// A junction or symlink: something that points elsewhere rather than holding the data.
    pub fn is_reparse(md: &Metadata) -> bool {
        md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    /// Hidden *and* system: Windows' marker for "this is the OS's business, not the user's".
    pub fn is_hidden_system(md: &Metadata) -> bool {
        let a = md.file_attributes();
        a & FILE_ATTRIBUTE_HIDDEN != 0 && a & FILE_ATTRIBUTE_SYSTEM != 0
    }
}

#[cfg(unix)]
mod attrs {
    use std::fs::Metadata;

    /// There are no junctions here, so a symlink is the whole story.
    pub fn is_reparse(md: &Metadata) -> bool {
        md.file_type().is_symlink()
    }

    /// The closest thing to Windows' hidden+system is the Finder "hidden" flag. The dot-prefix
    /// half of the rule lives in `is_hidden_name`, because a name is all the caller has in some
    /// places. `st_flags` is a BSD extension, not part of the portable Unix trait.
    #[cfg(target_os = "macos")]
    pub fn is_hidden_system(md: &Metadata) -> bool {
        use std::os::macos::fs::MetadataExt;
        const UF_HIDDEN: u32 = 0x8000;
        md.st_flags() & UF_HIDDEN != 0
    }

    #[cfg(not(target_os = "macos"))]
    pub fn is_hidden_system(_md: &Metadata) -> bool {
        false
    }
}

pub use attrs::{is_hidden_system, is_reparse};

/// Hidden by name rather than by flag: a leading dot. On Windows nothing is hidden this way,
/// but plenty of Unix tools still litter `~` with dotfiles that are none of our business.
pub fn is_hidden_name(name: &str) -> bool {
    cfg!(unix) && name.starts_with('.')
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

/// Whether a folder tree holds no files at all, and its usage if it doesn't.
///
/// `usage` answers this too, but only after measuring every file in the tree — and the answer is
/// settled by the first one found. For a folder with a lot in it that is the difference between a
/// full recursive walk and a single `read_dir`.
///
/// `None` for anything that is not an empty folder: a file, a reparse point, or a tree with a file
/// somewhere inside. Reparse points within the tree are skipped rather than counted, exactly as
/// `usage` skips them.
pub fn empty_tree(path: &Path) -> Option<Usage> {
    let md = fs::symlink_metadata(path).ok()?;
    if is_reparse(&md) || !md.is_dir() {
        return None;
    }
    let mut u = Usage { newest: mtime(&md), is_dir: true, ..Usage::default() };
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let Ok(md) = entry.metadata() else { continue };
            if is_reparse(&md) {
                continue;
            }
            if !md.is_dir() {
                return None;
            }
            u.newest = u.newest.max(mtime(&md));
            stack.push(entry.path());
        }
    }
    Some(u)
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

/// Directory extensions macOS treats as a single document rather than a folder: app bundles,
/// frameworks, plug-ins, photo libraries. Their insides are an app's private business.
#[cfg(target_os = "macos")]
const PACKAGE_EXTS: [&str; 14] = [
    "app", "framework", "bundle", "plugin", "kext", "appex", "xpc", "docset", "photoslibrary", "fcpbundle", "logicx", "band", "rtfd", "pkg",
];

/// A folder that should be treated as opaque: never walked into, never reported piecemeal.
///
/// Without this, the version signal happily proposes deleting
/// `Foo.app/Contents/Frameworks/Bar.framework/Versions/A` — part of an installed app. It also
/// keeps the scan from crawling tens of thousands of files inside every bundle.
pub fn is_package(dir: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        dir.extension().is_some_and(|e| PACKAGE_EXTS.contains(&e.to_string_lossy().to_lowercase().as_str()))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = dir;
        false
    }
}

/// Visit directories breadth-first down to `max_depth` (root is depth 0). `visit` returns
/// false to stop descending into that directory. Packages are never descended into.
pub fn walk_dirs(root: &Path, max_depth: usize, mut visit: impl FnMut(&Path, usize) -> bool) {
    let mut queue = std::collections::VecDeque::from([(root.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = queue.pop_front() {
        if !visit(&dir, depth) || depth >= max_depth {
            continue;
        }
        for child in child_dirs(&dir) {
            if !is_package(&child) {
                queue.push_back((child, depth + 1));
            }
        }
    }
}

pub fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn lower(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

/// The path separator, as a byte and as a char, for the OS this was built for.
const SEP: char = std::path::MAIN_SEPARATOR;

/// True if `inner` is `outer` or somewhere below it (case-insensitive).
///
/// Callers lowercase both sides first. On macOS that is right for the default case-insensitive
/// APFS volume, and on a case-sensitive one it can only ever match *more* than it should —
/// which, for the protection and already-covered checks this backs, is the safe direction.
pub fn is_within(inner: &str, outer: &str) -> bool {
    let inner = inner.trim_end_matches(SEP);
    let outer = outer.trim_end_matches(SEP);
    // `outer` trimmed to nothing means the filesystem root, which contains everything.
    if outer.is_empty() {
        return inner.starts_with(SEP);
    }
    inner == outer || (inner.len() > outer.len() && inner.starts_with(outer) && inner.as_bytes()[outer.len()] == SEP as u8)
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

    /// An absolute path in this platform's spelling: `c:\a\b` or `/a/b`.
    fn abs(parts: &[&str]) -> String {
        let root = if cfg!(windows) { "c:" } else { "" };
        format!("{root}{SEP}{}", parts.join(&SEP.to_string()))
    }

    #[test]
    fn within() {
        assert!(is_within(&abs(&["a", "b"]), &abs(&["a"])));
        assert!(is_within(&abs(&["a"]), &abs(&["a"])));
        assert!(!is_within(&abs(&["ab"]), &abs(&["a"])));
        // A trailing separator on the outer path must not change the answer.
        assert!(is_within(&abs(&["a", "b"]), &format!("{}{SEP}", abs(&["a"]))));
    }

    /// `empty_tree` decides whether a folder is offered for removal, so a wrong "yes" would mean
    /// deleting something with files in it. It has to agree with `usage` on every shape.
    #[test]
    fn empty_tree_agrees_with_usage() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        let bare = root.join("bare");
        fs::create_dir(&bare).unwrap();
        assert!(empty_tree(&bare).is_some(), "an empty folder is empty");

        let nested = root.join("nested");
        fs::create_dir_all(nested.join("a/b/c")).unwrap();
        assert!(empty_tree(&nested).is_some(), "folders all the way down and no files is still empty");

        let shallow = root.join("shallow");
        fs::create_dir(&shallow).unwrap();
        fs::write(shallow.join("f.txt"), b"x").unwrap();
        assert!(empty_tree(&shallow).is_none(), "a file at the top means not empty");

        let deep = root.join("deep");
        fs::create_dir_all(deep.join("a/b/c")).unwrap();
        fs::write(deep.join("a/b/c/f.txt"), b"x").unwrap();
        assert!(empty_tree(&deep).is_none(), "a file buried deep still means not empty");

        let file = root.join("plain.txt");
        fs::write(&file, b"x").unwrap();
        assert!(empty_tree(&file).is_none(), "a file is not an empty folder");

        assert!(empty_tree(&root.join("missing")).is_none(), "nothing there is not an empty folder");

        // The whole point of the function: the same answer as `usage`, without measuring.
        for p in [&bare, &nested, &shallow, &deep, &file] {
            assert_eq!(empty_tree(p).is_some(), usage(p).files == 0 && p.is_dir(), "disagreed about {p:?}");
        }
    }

}
