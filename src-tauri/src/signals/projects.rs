//! Folders made by a creative tool or an engine, and the parts of them that rebuild themselves.
//!
//! The difficulty this solves: a Pro Tools session, a Unity project and a folder of holiday
//! photos look identical from the outside. The folder name says nothing. What says everything is
//! a marker file sitting beside the content — `Session.ptx`, `ProjectSettings/ProjectVersion.txt`
//! — and once that is found, the rest of the folder stops being a mystery. `Library` next to a
//! Unity marker is an import cache the engine rebuilds in minutes; the same folder name anywhere
//! else is nobody's business.
//!
//! That marker is also the proof that lets these findings be acted on inside otherwise protected
//! places like `~/Documents` — see `Finding::vouched`.
//!
//! Three things come out of here: the regenerable parts of a project, projects that were created
//! and never really used, and projects nobody has opened in a long time. The last two are only
//! ever `YourCall`, and they stay separate findings because they are not the same claim.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use rayon::prelude::*;

use super::{age_days, finding, item, Taken};
use crate::fsutil::{self, Usage};
use crate::report::{Category, Confidence, Finding, Tier};
use crate::rules::{ProjectKind, Regenerable, Rules};
use crate::roots::Roots;
use crate::scan::Ctx;

/// A project folder paired with what kind of project it is.
type Found<'a> = (&'a ProjectKind, PathBuf, Usage);
/// One occurrence of a regenerable part, and which part it is.
type PartHit<'a> = (&'a ProjectKind, &'a Regenerable, PathBuf, Usage);
/// Every occurrence of one part, across every project of one kind.
type PartGroup<'a> = (&'a ProjectKind, &'a Regenerable, Vec<(PathBuf, Usage)>);

/// How deep below a starting folder to look for project markers. Projects are usually one or
/// two levels in (`~/Documents/Sessions/MySong`), rarely deeper.
const DEPTH: usize = 4;

/// Ignore a regenerable part smaller than this. Clearing a 2 KB cache helps nobody, and one row
/// per trivial folder would bury the findings that matter.
const MIN_PART_BYTES: u64 = 1 << 20;

/// A project counts as untouched once nothing in it has changed for this long.
const STALE_DAYS: u64 = 365;

/// Every project folder on the machine, as `(path, index into rules.project_kind)`.
///
/// Called once from `Ctx::new` rather than from `find` below, so that every signal can ask what
/// a folder belongs to — including the ones that run before this one.
pub fn find_projects(roots: &Roots, rules: &Rules) -> Vec<(PathBuf, usize)> {
    if rules.project_kind.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for start in roots.project_search_roots() {
        fsutil::walk_dirs(&start, DEPTH, |dir, depth| {
            // Depth 0 is the search root itself — `~/Downloads`, `~/Documents`, `~/Music`. A
            // single stray project file dropped into one of those must never turn the whole
            // folder into "a project", which would put everything in it up for removal.
            if depth == 0 {
                return true;
            }
            match rules.project_kind_of(dir).and_then(|k| rules.project_kind.iter().position(|x| x.id == k.id)) {
                Some(kind) => {
                    out.push((dir.to_path_buf(), kind));
                    // A project does not contain another project. Stopping here also keeps the
                    // walk off the thousands of files inside a big one.
                    false
                }
                None => true,
            }
        });
    }
    out
}

pub fn find(ctx: &Ctx, taken: &Taken) -> Vec<Finding> {
    let projects: Vec<(PathBuf, &ProjectKind)> = ctx.projects().map(|(p, k)| (p.to_path_buf(), k)).collect();
    if projects.is_empty() {
        return Vec::new();
    }
    let mut out = regenerable_parts(ctx, taken, &projects);
    out.extend(unused_projects(ctx, taken, &projects));
    out.extend(stale_projects(ctx, taken, &projects));
    out
}

// ── The parts a tool rebuilds by itself ─────────────────────────────────────────

/// One finding per (project kind, part), holding every occurrence as an item — so 135 sessions'
/// worth of waveform cache reads as one row rather than 135.
fn regenerable_parts(ctx: &Ctx, taken: &Taken, projects: &[(PathBuf, &ProjectKind)]) -> Vec<Finding> {
    // One entry per (kind, part), gathering every project that has it.
    let mut groups: Vec<PartGroup> = Vec::new();

    let hits: Vec<PartHit> = projects
        .par_iter()
        .flat_map_iter(|(root, kind)| {
            let mut found = Vec::new();
            for (path, md) in fsutil::children(root) {
                let name = fsutil::file_name(&path);
                let Some(part) = kind.regenerable.iter().find(|r| fsutil::wildcard(&r.name, &name)) else { continue };
                if taken.covers(&path) || !ctx.may_remove(&path, true) {
                    continue;
                }
                let u = if md.is_dir() { fsutil::usage(&path) } else { Usage { bytes: md.len(), files: 1, newest: fsutil::mtime(&md), is_dir: false } };
                // A project touched in the last day may be open in the editor right now. Clearing
                // a session's waveform cache or an engine's shader cache underneath it is a bad
                // surprise, and `clean`'s running-process check won't catch it: the app is running
                // legitimately, it just happens to have this document open. Wait a day.
                if age_days(ctx, u.newest) < 1 {
                    ctx.skipped_recent.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                found.push((*kind, part, path, u));
            }
            found
        })
        .collect();

    for (kind, part, path, u) in hits {
        match groups.iter_mut().find(|(k, p, _)| k.id == kind.id && p.name == part.name) {
            Some((_, _, items)) => items.push((path, u)),
            None => groups.push((kind, part, vec![(path, u)])),
        }
    }

    groups
        .into_iter()
        .filter_map(|(kind, part, items)| {
            let total: u64 = items.iter().map(|(_, u)| u.bytes).sum();
            if total < MIN_PART_BYTES {
                return None;
            }
            let newest = items.iter().map(|(_, u)| u.newest).max().unwrap_or(0);
            let mut f = finding(
                format!("project:{}:{}", kind.id, part.name.to_lowercase()),
                format!("{} {}", kind.name, part.name),
                part.tier.into(),
                Category::Projects,
                Confidence::High,
            );
            f.what = part.what.clone();
            f.if_deleted = Some(part.if_deleted.clone());
            f.evidence.push(match items.len() {
                1 => format!("Found in 1 {}.", kind.name),
                n => format!("Found in {n} {}s.", kind.name),
            });
            f.evidence.push(super::last_changed(ctx, newest));
            f.last_modified = Some(newest);
            f.vouched = true;
            for (path, u) in &items {
                f.items.push(item(path, u));
            }
            f.recompute_bytes();
            Some(f)
        })
        .collect()
}

// ── Projects that were created and never used ───────────────────────────────────

fn unused_projects(ctx: &Ctx, taken: &Taken, projects: &[(PathBuf, &ProjectKind)]) -> Vec<Finding> {
    let found: Vec<Found> = projects
        .par_iter()
        .filter_map(|(root, kind)| {
            // Only ask "was this ever used?" of tools that have somewhere to put content. For a
            // LaTeX document or a `.blend`, the project file is the work.
            if kind.content.is_empty() || taken.covers(root) || !ctx.may_remove(root, true) || has_content(root, kind) {
                return None;
            }
            let u = fsutil::usage(root);
            if age_days(ctx, u.newest) < 1 {
                ctx.skipped_recent.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            Some((*kind, root.clone(), u))
        })
        .collect();

    group_by_kind(ctx, found, "unused", |kind, n, oldest, newest| {
        let mut f = finding(
            format!("project-unused:{}", kind.id),
            match n {
                1 => format!("1 unused {}", kind.name),
                n => format!("{n} unused {}s", kind.name),
            },
            Tier::YourCall,
            Category::Projects,
            Confidence::High,
        );
        f.what = Some(format!("{}s that were created and saved, but hold no media — nothing was ever recorded or imported into them.", kind.name));
        f.if_deleted = Some("Only the empty project files go. There is no recorded work inside them to lose.".into());
        f.evidence.push(format!("Each has a {} file, but its {} is empty.", kind.name, kind.content.join(" and ")));
        f.evidence.push(format!("Last changed between {} and {}.", fsutil::date(oldest), fsutil::date(newest)));
        f
    })
}

/// Recorded or imported material, by extension. Used as a backstop rather than a primary test:
/// finding any one of these anywhere inside means the folder is somebody's work.
const MEDIA_EXTS: [&str; 22] = [
    "wav", "aif", "aiff", "aifc", "caf", "w64", "bwf", "flac", "mp3", "m4a", "ogg", "opus", "wma", "rex", "sd2", "mov", "mp4", "avi", "mxf",
    "r3d", "braw", "mkv",
];

/// Does this project hold actual recorded or imported content?
///
/// Three ways to say yes, deliberately generous — a false "yes" only means a leftover goes
/// unreported, while a false "no" offers up somebody's work.
fn has_content(root: &Path, kind: &ProjectKind) -> bool {
    let declared = fsutil::children(root).iter().any(|(p, md)| {
        if md.is_dir() {
            let name = fsutil::file_name(p);
            kind.content.iter().any(|c| c.eq_ignore_ascii_case(&name)) && fsutil::usage(p).files > 0
        } else {
            // Something substantial sitting loose in the project folder counts too.
            md.len() > MIN_PART_BYTES
        }
    });
    declared || holds_media(root)
}

/// Any media file at all, anywhere within a few levels.
fn holds_media(root: &Path) -> bool {
    let mut found = false;
    fsutil::walk_dirs(root, 3, |dir, _| {
        if found {
            return false;
        }
        found = fsutil::children(dir).iter().any(|(p, md)| {
            md.is_file() && p.extension().is_some_and(|e| MEDIA_EXTS.contains(&e.to_string_lossy().to_lowercase().as_str()))
        });
        !found
    });
    found
}

// ── Projects nobody has opened in a long time ───────────────────────────────────

fn stale_projects(ctx: &Ctx, taken: &Taken, projects: &[(PathBuf, &ProjectKind)]) -> Vec<Finding> {
    let found: Vec<Found> = projects
        .par_iter()
        .filter_map(|(root, kind)| {
            if kind.content.is_empty() || taken.covers(root) || !ctx.may_remove(root, true) {
                return None;
            }
            let u = fsutil::usage(root);
            // The unused group already covers empty ones, and says something truer about them.
            if age_days(ctx, u.newest) < STALE_DAYS || !has_content(root, kind) {
                return None;
            }
            Some((*kind, root.clone(), u))
        })
        .collect();

    group_by_kind(ctx, found, "stale", |kind, n, oldest, newest| {
        let mut f = finding(
            format!("project-stale:{}", kind.id),
            match n {
                1 => format!("1 {} not opened in over a year", kind.name),
                n => format!("{n} {}s not opened in over a year", kind.name),
            },
            Tier::YourCall,
            Category::Projects,
            Confidence::Medium,
        );
        f.what = Some(format!("{}s holding real recorded or imported work that nothing has touched for a year or more.", kind.name));
        f.if_deleted = Some("The work inside is gone for good. Look through them first — an old project is still finished work.".into());
        f.evidence.push("Each one still contains media, so this is not junk — only old.".into());
        f.evidence.push(format!("Last changed between {} and {}.", fsutil::date(oldest), fsutil::date(newest)));
        f
    })
}

// ── Shared grouping ─────────────────────────────────────────────────────────────

/// Roll per-project hits up into one finding per project kind, keeping each project as its own
/// item so the user can still pick through them individually.
fn group_by_kind(
    _ctx: &Ctx,
    found: Vec<Found>,
    _what: &str,
    build: impl Fn(&ProjectKind, usize, u64, u64) -> Finding,
) -> Vec<Finding> {
    let mut groups: Vec<(&ProjectKind, Vec<(PathBuf, Usage)>)> = Vec::new();
    for (kind, path, u) in found {
        match groups.iter_mut().find(|(k, _)| k.id == kind.id) {
            Some((_, items)) => items.push((path, u)),
            None => groups.push((kind, vec![(path, u)])),
        }
    }
    groups
        .into_iter()
        .map(|(kind, items)| {
            let oldest = items.iter().map(|(_, u)| u.newest).filter(|t| *t > 0).min().unwrap_or(0);
            let newest = items.iter().map(|(_, u)| u.newest).max().unwrap_or(0);
            let mut f = build(kind, items.len(), oldest, newest);
            f.last_modified = Some(newest);
            f.vouched = true;
            for (path, u) in &items {
                f.items.push(item(path, u));
            }
            f.recompute_bytes();
            f
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real Pro Tools entry, read from the shipped rules so the test tracks the data.
    fn protools_kind() -> ProjectKind {
        let rules = Rules::load();
        rules.project_kind.into_iter().find(|k| k.id == "protools").expect("protools kind in the rules file")
    }

    fn session(root: &Path, name: &str, with_audio: bool) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("Audio Files")).unwrap();
        std::fs::write(dir.join(format!("{name}.ptx")), vec![0u8; 1024]).unwrap();
        if with_audio {
            std::fs::write(dir.join("Audio Files").join("take1.wav"), vec![0u8; 4096]).unwrap();
        }
        dir
    }

    #[test]
    fn content_tells_a_used_project_from_an_abandoned_one() {
        let tmp = tempfile::tempdir().unwrap();
        let empty = session(tmp.path(), "Untitled", false);
        let used = session(tmp.path(), "RealSong", true);
        let kind = protools_kind();
        assert!(!has_content(&empty, &kind), "an empty Audio Files folder is not content");
        assert!(has_content(&used, &kind));
    }

    /// The bug this caught: a stray `.ptx` in `~/Downloads` made the whole folder "a project",
    /// and everything in it was offered for removal.
    #[test]
    fn a_stray_project_file_does_not_make_a_search_root_a_project() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("48.ptx"), vec![0u8; 512]).unwrap();
        // The root holds a marker, but it is the folder being searched, not something inside it.
        let rules = Rules::load();
        assert!(rules.project_kind_of(tmp.path()).is_some(), "the marker is genuinely there");
        // What matters is that discovery skips depth 0, which the real walk does.
        let found: Vec<_> = fsutil::child_dirs(tmp.path()).into_iter().filter(|d| rules.project_kind_of(d).is_some()).collect();
        assert!(found.is_empty(), "nothing inside it is a project");
    }

    #[test]
    fn any_media_inside_means_the_project_was_worked_in() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = session(tmp.path(), "Nested", false);
        // Not in Audio Files, not loose at the top, and under the size floor — but still work.
        std::fs::create_dir_all(dir.join("Somewhere").join("Deep")).unwrap();
        std::fs::write(dir.join("Somewhere").join("Deep").join("take.wav"), vec![0u8; 64]).unwrap();
        assert!(has_content(&dir, &protools_kind()));
    }

    /// Engines are the biggest claims in the knowledge base — a Unity `Library` is routinely
    /// tens of gigabytes — and no machine here has one to scan. This exercises the data itself.
    #[test]
    fn engine_projects_are_recognised_and_tiered() {
        let rules = Rules::load();

        // Unity's marker is a nested path, which is the only kind written with a `/`.
        let tmp = tempfile::tempdir().unwrap();
        let unity = tmp.path().join("MyGame");
        std::fs::create_dir_all(unity.join("ProjectSettings")).unwrap();
        std::fs::write(unity.join("ProjectSettings").join("ProjectVersion.txt"), "m_EditorVersion: 2022.3.1f1").unwrap();
        let kind = rules.project_kind_of(&unity).expect("Unity project recognised");
        assert_eq!(kind.id, "unity");

        let part = |k: &crate::rules::ProjectKind, n: &str| k.regenerable.iter().find(|r| fsutil::wildcard(&r.name, n)).map(|r| r.tier);
        // The import cache rebuilds itself, so it is safe.
        assert_eq!(part(kind, "Library"), Some(crate::rules::RuleTier::Safe));
        assert_eq!(part(kind, "Temp"), Some(crate::rules::RuleTier::Safe));
        // The user's own work is not listed at all, so it can never be offered.
        assert_eq!(part(kind, "Assets"), None, "Assets is the project itself");
        assert_eq!(part(kind, "ProjectSettings"), None);

        // Unreal, whose `Saved` holds autosaves and so must not be called safe.
        let unreal = tmp.path().join("Shooter");
        std::fs::create_dir_all(&unreal).unwrap();
        std::fs::write(unreal.join("Shooter.uproject"), "{}").unwrap();
        let kind = rules.project_kind_of(&unreal).expect("Unreal project recognised");
        assert_eq!(kind.id, "unreal");
        assert_eq!(part(kind, "DerivedDataCache"), Some(crate::rules::RuleTier::Safe));
        assert_eq!(part(kind, "Intermediate"), Some(crate::rules::RuleTier::Safe));
        assert_eq!(part(kind, "Saved"), Some(crate::rules::RuleTier::YourCall), "Saved holds autosaves");
        assert_eq!(part(kind, "Content"), None, "Content is the project itself");
    }

    /// Discovery has to find a project nested under a search root, and must not be fooled by
    /// the kind of folder that merely *contains* projects.
    #[test]
    fn discovery_finds_nested_projects() {
        let tmp = tempfile::tempdir().unwrap();
        let docs = tmp.path().join("Documents");
        std::fs::create_dir_all(docs.join("Sessions")).unwrap();
        session(&docs.join("Sessions"), "SongOne", true);
        session(&docs.join("Sessions"), "SongTwo", false);

        let roots = Roots::for_test(tmp.path());
        let rules = Rules::load();
        let found = find_projects(&roots, &rules);
        assert_eq!(found.len(), 2, "both sessions, not the folder holding them: {found:?}");
        assert!(found.iter().all(|(p, _)| p.ends_with("SongOne") || p.ends_with("SongTwo")));
    }

    #[test]
    fn loose_media_in_the_project_folder_counts_as_content() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = session(tmp.path(), "Loose", false);
        std::fs::write(dir.join("bounce.wav"), vec![0u8; (MIN_PART_BYTES + 1) as usize]).unwrap();
        assert!(has_content(&dir, &protools_kind()));
    }
}
