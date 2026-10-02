//! Each signal looks at one kind of clutter and returns findings with plain-language
//! evidence. Signals run in order; later ones skip anything an earlier one already covered.

mod cachedirs;
mod dev;
mod downloads;
mod empty;
mod games;
mod known;
mod orphans;
mod projects;
mod stray;
mod temp;
mod versions;

use std::path::Path;

use regex::Regex;

use crate::fsutil::{self, Usage};
use crate::report::{Category, Confidence, Finding, Item, Tier};
use crate::scan::Ctx;

/// Lowercased paths already covered by a finding.
#[derive(Default)]
pub struct Taken(Vec<String>);

impl Taken {
    pub fn covers(&self, p: &Path) -> bool {
        let l = fsutil::lower(p);
        self.0.iter().any(|t| fsutil::is_within(&l, t))
    }

    fn add(&mut self, findings: &[Finding]) {
        for f in findings {
            self.0.extend(f.items.iter().map(|i| i.path.to_lowercase()));
        }
    }
}

type Signal = fn(&Ctx, &Taken) -> Vec<Finding>;

/// Signals in priority order: curated knowledge first, then targeted signals, then the
/// general "nothing owns this" style signals.
const SIGNALS: [Signal; 12] = [
    known::find,
    games::find_steam,
    dev::find,
    // After `dev`, so a git repo that also holds a project file is described in git terms.
    // Before `orphans`, `find_saves`, `empty` and `stray`, whose guesses about these folders
    // are vaguer — running first means `Taken` mutes them.
    projects::find,
    downloads::find,
    temp::find,
    versions::find,
    orphans::find,
    games::find_saves,
    empty::find,
    stray::find,
    cachedirs::find,
];

pub use projects::find_projects;

pub fn run_all(ctx: &Ctx) -> Vec<Finding> {
    let mut taken = Taken::default();
    let mut all = Vec::new();
    for signal in SIGNALS {
        let found = signal(ctx, &taken);
        taken.add(&found);
        all.extend(found);
    }
    all
}

pub fn item(path: &Path, u: &Usage) -> Item {
    Item { path: path.to_string_lossy().into_owned(), bytes: u.bytes, files: u.files, is_dir: u.is_dir }
}

pub fn finding(id: impl Into<String>, title: impl Into<String>, tier: Tier, category: Category, confidence: Confidence) -> Finding {
    Finding {
        id: id.into(),
        title: title.into(),
        tier,
        category,
        confidence,
        what: None,
        if_deleted: None,
        evidence: Vec::new(),
        items: Vec::new(),
        vouched: false,
        bytes: 0,
        last_modified: None,
    }
}

pub fn last_changed(ctx: &Ctx, newest: u64) -> String {
    format!("Last changed {}.", fsutil::when(ctx.now, newest))
}

pub fn age_days(ctx: &Ctx, newest: u64) -> u64 {
    ctx.now.saturating_sub(newest) / fsutil::DAY
}

const CONFIG_EXTS: [&str; 10] = ["txt", "json", "ini", "cfg", "conf", "yml", "yaml", "toml", "xml", "config"];

/// What an absolute path looks like in a config file.
///
/// On macOS this is anchored on the handful of real top-level directories rather than a bare
/// `/`, which would match every URL path and every passing mention of `/usr/bin`.
#[cfg(windows)]
const ABS_PATH_RE: &str = r#"(?i)\b[a-z]:\\[^\r\n"'<>|*?]{2,240}"#;
#[cfg(not(windows))]
const ABS_PATH_RE: &str = r#"(?i)/(?:users|applications|library|opt|private|var|usr/local|etc)/[^\r\n"'<>|*?:]{2,240}"#;

/// Undo JSON/INI escaping so the paths in the text can be compared against the real thing.
fn normalize_refs(text: &str) -> String {
    let unescaped = text.replace(r"\\", r"\");
    // Windows config files write either separator and mean the same folder. On macOS `\` is a
    // legal filename character, so rewriting it would corrupt every POSIX path in the file.
    if cfg!(windows) {
        unescaped.replace('/', r"\")
    } else {
        unescaped
    }
}

/// Small config files inside `dir` that point at absolute paths which no longer exist,
/// e.g. conda's environments.txt pointing at a deleted Miniconda folder.
pub fn dead_refs(dir: &Path) -> Vec<String> {
    let re = Regex::new(ABS_PATH_RE).unwrap();
    let mut out = Vec::new();
    let mut files_read = 0;
    fsutil::walk_dirs(dir, 2, |d, _| {
        for (file, md) in fsutil::children(d) {
            if out.len() >= 3 || files_read >= 40 {
                return false;
            }
            let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            if !md.is_file() || md.len() > 64 * 1024 || !CONFIG_EXTS.contains(&ext.as_str()) {
                continue;
            }
            files_read += 1;
            let Ok(bytes) = std::fs::read(&file) else { continue };
            let text = normalize_refs(&String::from_utf8_lossy(&bytes));
            let mut any_alive = false;
            let mut missing = Vec::new();
            for m in re.find_iter(&text) {
                let p = m.as_str().trim_end_matches([' ', ',', ';', ']', ')', '}', '.']);
                if Path::new(p).exists() {
                    any_alive = true;
                } else {
                    missing.push(p.to_string());
                }
            }
            // Only trust it when nothing the file mentions still exists.
            if !any_alive {
                if let Some(first) = missing.first() {
                    out.push(format!("{} points to {}, which no longer exists.", fsutil::file_name(&file), first));
                }
            }
        }
        true
    });
    out
}

const SAVE_DIRS: [&str; 6] = ["savegames", "saves", "savedata", "savegame", "save", "savedgames"];
const SAVE_EXTS: [&str; 4] = ["sav", "save", "savegame", "sl2"];

/// Does this folder look like it holds game save files?
pub fn looks_like_saves(dir: &Path) -> bool {
    let mut found = false;
    fsutil::walk_dirs(dir, 4, |d, _| {
        if found {
            return false;
        }
        if SAVE_DIRS.contains(&fsutil::file_name(d).to_lowercase().as_str()) {
            found = true;
            return false;
        }
        found = fsutil::children(d).iter().any(|(f, md)| {
            md.is_file() && f.extension().is_some_and(|e| SAVE_EXTS.contains(&e.to_string_lossy().to_lowercase().as_str()))
        });
        !found
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dead absolute path, spelled the way this platform's config files would.
    #[cfg(windows)]
    const DEAD: [&str; 2] = [r"C:\Users\nobody\miniconda3", r"C:\Users\nobody\miniconda3\envs\x"];
    #[cfg(not(windows))]
    const DEAD: [&str; 2] = ["/Users/nobody/miniconda3", "/Users/nobody/miniconda3/envs/x"];

    #[test]
    fn finds_dead_references() {
        let dir = tempfile::tempdir().unwrap();
        let text = format!("{}\r\n{}\r\n", DEAD[0], DEAD[1]);
        std::fs::write(dir.path().join("environments.txt"), text).unwrap();
        let refs = dead_refs(dir.path());
        assert_eq!(refs.len(), 1);
        assert!(refs[0].contains("miniconda3"), "{refs:?}");
    }

    #[test]
    fn live_reference_is_not_dead() {
        let dir = tempfile::tempdir().unwrap();
        // JSON escapes a backslash, so a Windows path is doubled in the file's bytes.
        let alive = dir.path().to_string_lossy().replace('\\', "\\\\");
        let dead = DEAD[0].replace('\\', "\\\\");
        std::fs::write(dir.path().join("config.json"), format!("{{\"a\": \"{alive}\", \"b\": \"{dead}\"}}")).unwrap();
        assert!(dead_refs(dir.path()).is_empty());
    }

    #[test]
    fn detects_saves() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Game").join("Saved").join("SaveGames")).unwrap();
        assert!(looks_like_saves(dir.path()));
        let other = tempfile::tempdir().unwrap();
        std::fs::write(other.path().join("settings.ini"), "x").unwrap();
        assert!(!looks_like_saves(other.path()));
    }
}
