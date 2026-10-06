//! Large folders inside git repos that git ignores: build output and installed dependencies.

use std::path::{Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use rayon::prelude::*;

use super::{finding, item, last_changed, Taken};
use crate::fsutil;
use crate::report::{Category, Confidence, Finding, Tier};
use crate::scan::Ctx;

const MIN_BYTES: u64 = 20 << 20;
const NEVER_DESCEND: [&str; 6] = ["node_modules", ".git", "appdata", "dist", "build", "target"];
/// Folders that only exist, or only matter, on macOS. Kept separate so the Windows scan looks
/// in exactly the places it always has.
const NEVER_DESCEND_MAC: [&str; 3] = ["pods", "deriveddata", "library"];

/// Top-level home folders that never hold source.
const NOT_PROJECTS: [&str; 2] = ["appdata", "onedrive"];
/// `Library` matters most here: it is not dot-prefixed, holds hundreds of deep per-app trees,
/// and crawling it looking for `.git` is both pointless and slow.
const NOT_PROJECTS_MAC: [&str; 6] = ["library", "applications", "pictures", "movies", "music", "public"];

/// Is this a folder the repo search should skip entirely?
fn not_a_project(name: &str) -> bool {
    NOT_PROJECTS.contains(&name) || (cfg!(target_os = "macos") && NOT_PROJECTS_MAC.contains(&name))
}

/// Is this a folder the repo search should not descend into?
fn never_descend(name: &str) -> bool {
    NEVER_DESCEND.contains(&name) || (cfg!(target_os = "macos") && NEVER_DESCEND_MAC.contains(&name))
}

pub fn find(ctx: &Ctx, taken: &Taken) -> Vec<Finding> {
    let repos = find_repos(ctx);
    let candidates: Vec<(PathBuf, PathBuf)> = repos
        .iter()
        .filter_map(|repo| Some((repo, load_gitignore(repo)?)))
        .flat_map(|(repo, gi)| {
            fsutil::child_dirs(repo)
                .into_iter()
                .filter(move |d| fsutil::file_name(d) != ".git" && gi.matched(d, true).is_ignore())
                .map(move |d| (repo.clone(), d))
        })
        .filter(|(_, d)| !taken.covers(d))
        // Vouched, because the repo's own `.gitignore` is the proof that gets these past
        // `protected` — but the name guard underneath still applies. Checked here rather than
        // left to `scan::run`'s backstop so nothing off limits is measured in the first place,
        // the same way `empty`, `projects` and `cachedirs` do it.
        .filter(|(_, d)| ctx.may_remove(d, true))
        .collect();

    ctx.progress.units(candidates.len());
    candidates
        .par_iter()
        .filter_map(|(repo, dir)| {
            ctx.progress.examining(dir);
            let u = fsutil::usage(dir);
            if u.bytes < MIN_BYTES {
                return None;
            }
            let name = fsutil::file_name(dir);
            let repo_name = fsutil::file_name(repo);
            let restore = ctx.rules.restore_dir.iter().find(|r| r.name.eq_ignore_ascii_case(&name));
            // Recognised as restorable, but the project no longer records what to restore from —
            // so this folder is the only copy. Leave it alone rather than offer it with an
            // instruction that cannot work.
            if restore.is_some_and(|r| !r.restorable_from(repo)) {
                return None;
            }
            let is_build = ctx.rules.build_dirs.iter().any(|b| b.eq_ignore_ascii_case(&name));
            // If this repo is also a recognised project, that knowledge is better than ours: a
            // Unity `Library` is an import cache the engine rebuilds, not mystery local data.
            // Reading the same table the projects signal does keeps the two from describing the
            // same folder differently depending on whether it happens to be under git.
            let as_project = ctx
                .project_of(repo)
                .and_then(|(_, kind)| kind.regenerable.iter().find(|r| fsutil::wildcard(&r.name, &name)));
            let (tier, what, if_deleted) = if let Some(part) = as_project {
                (
                    part.tier.into(),
                    part.what.clone().unwrap_or_else(|| "Output the project's tools rebuild by themselves.".to_string()),
                    part.if_deleted.clone(),
                )
            } else if let Some(r) = restore {
                (Tier::YourCall, "Dependencies installed for this project.".to_string(), r.how.clone())
            } else if is_build {
                (Tier::Safe, "Build output generated from the project's source.".to_string(), "Recreated the next time you build the project.".to_string())
            } else {
                // Nothing recognises this folder, so there is no telling what is in it. The name
                // guard in `refuse_location` only ever sees the folder's own name, so look one
                // level in: if it holds something that looks like credentials or local settings,
                // don't offer it at all.
                if holds_secrets_inside(ctx, dir) {
                    return None;
                }
                (
                    Tier::YourCall,
                    "Local files the project keeps out of git.".to_string(),
                    "Can't be recreated from the repo. Check it isn't data you need (downloaded models, databases, settings).".to_string(),
                )
            };
            // A project's own part belongs with the other project findings, whether or not it is under git.
            let category = if as_project.is_some() { Category::Projects } else { Category::Dev };
            let mut f = finding(format!("dev:{}", fsutil::lower(dir)), format!("{repo_name}{}{name}", std::path::MAIN_SEPARATOR), tier, category, Confidence::High);
            f.what = Some(what);
            f.if_deleted = Some(if_deleted);
            f.evidence.push(format!("The {repo_name} repo's .gitignore excludes it, so it isn't part of the project's source."));
            f.evidence.push(last_changed(ctx, u.newest));
            f.last_modified = Some(u.newest);
            // The repo's own .gitignore names this folder, so the project is saying it isn't
            // source. Repos live wherever the user keeps them, which is often Documents or
            // Desktop — both protected — so without the vouch those findings vanish.
            f.vouched = true;
            f.items.push(item(dir, &u));
            f.recompute_bytes();
            Some(f)
        })
        .collect()
}

/// Does the top level of this folder hold something named like credentials or local settings?
///
/// Only asked of folders nothing recognises — a `build_dirs`, `restore_dir` or project part is
/// already known to be rebuildable output, and reading inside every one of those would be work for
/// no answer. One `read_dir`, after `fsutil::usage` has already walked the tree.
fn holds_secrets_inside(ctx: &Ctx, dir: &Path) -> bool {
    fsutil::children(dir).iter().any(|(p, _)| ctx.rules.is_never_name(&fsutil::file_name(p)))
}

/// Git repos within three levels of the home folder's top-level project folders.
fn find_repos(ctx: &Ctx) -> Vec<PathBuf> {
    let mut starts: Vec<PathBuf> = fsutil::child_dirs(&ctx.roots.home)
        .into_iter()
        .filter(|d| {
            let n = fsutil::file_name(d).to_lowercase();
            !n.starts_with('.') && !not_a_project(&n)
        })
        .collect();
    starts.extend(ctx.roots.dev_extra_starts());
    let mut repos = Vec::new();
    for start in starts {
        fsutil::walk_dirs(&start, 3, |dir, depth| {
            let name = fsutil::file_name(dir).to_lowercase();
            if depth > 0 && never_descend(&name) {
                return false;
            }
            if dir.join(".git").exists() {
                repos.push(dir.to_path_buf());
                return false;
            }
            true
        });
    }
    repos
}

fn load_gitignore(repo: &Path) -> Option<Gitignore> {
    let mut b = GitignoreBuilder::new(repo);
    let root_ignore = repo.join(".gitignore");
    if !root_ignore.is_file() {
        return None;
    }
    b.add(root_ignore);
    // Built up a component at a time: a literal `.git\info\exclude` is one filename containing
    // backslashes anywhere but Windows, so this never found anything on macOS.
    let exclude = repo.join(".git").join("info").join("exclude");
    if exclude.is_file() {
        b.add(exclude);
    }
    b.build().ok()
}

#[cfg(test)]
mod tests {
    use super::{find, load_gitignore, MIN_BYTES};
    use crate::inventory::Inventory;
    use crate::roots::Roots;
    use crate::rules::Rules;
    use crate::scan::Ctx;
    use crate::signals::Taken;

    #[test]
    fn matches_ignored_dirs() {
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join(".gitignore"), "node_modules/\n/dist\n").unwrap();
        for d in ["node_modules", "dist", "src"] {
            std::fs::create_dir(repo.path().join(d)).unwrap();
        }
        let gi = load_gitignore(repo.path()).unwrap();
        assert!(gi.matched(repo.path().join("node_modules"), true).is_ignore());
        assert!(gi.matched(repo.path().join("dist"), true).is_ignore());
        assert!(!gi.matched(repo.path().join("src"), true).is_ignore());
    }

    /// Local excludes live in `.git/info/exclude`, and a path built with backslashes found them
    /// only on Windows.
    #[test]
    fn local_excludes_are_read() {
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join(".gitignore"), "dist/\n").unwrap();
        std::fs::create_dir_all(repo.path().join(".git").join("info")).unwrap();
        std::fs::write(repo.path().join(".git").join("info").join("exclude"), "scratch/\n").unwrap();
        let gi = load_gitignore(repo.path()).unwrap();
        assert!(gi.matched(repo.path().join("scratch"), true).is_ignore(), "a locally excluded folder counts too");
    }

    /// A virtualenv is only "dependencies you can reinstall" while the project still records what
    /// was in it. With a lockfile it is hundreds of megabytes anyone can rebuild; without one the
    /// folder is the only copy, and offering it alongside "reinstall requirements" would be
    /// promising something that cannot be done.
    #[test]
    fn a_virtualenv_is_offered_only_when_it_can_be_rebuilt() {
        let home = tempfile::tempdir().unwrap();
        let big = vec![0u8; (MIN_BYTES + 1) as usize];
        let offered_for = |repo_name: &str, manifest: Option<&str>| {
            let repo = home.path().join(repo_name);
            std::fs::create_dir_all(repo.join(".git")).unwrap();
            std::fs::write(repo.join(".gitignore"), ".venv\n").unwrap();
            std::fs::create_dir(repo.join(".venv")).unwrap();
            std::fs::write(repo.join(".venv").join("payload.bin"), &big).unwrap();
            if let Some(m) = manifest {
                std::fs::write(repo.join(m), b"[project]\n").unwrap();
            }
            let ctx = Ctx::new(Roots::for_test(home.path()), Rules::load(), Inventory::default());
            find(&ctx, &Taken::default())
                .iter()
                .flat_map(|f| f.items.iter().map(|i| i.path.clone()))
                .any(|p| p.contains(repo_name) && p.ends_with(".venv"))
        };
        assert!(offered_for("locked", Some("uv.lock")), "a lockfile means it rebuilds, so offer it");
        assert!(offered_for("declared", Some("requirements.txt")), "a requirements file is enough too");
        assert!(!offered_for("orphaned", None), "nothing to reinstall from, so it must not be offered");
    }

    /// The whole point of the git-ignore signal is to offer build output — and never to offer
    /// somewhere secrets live, even though the repo's own `.gitignore` names both and the signal
    /// vouches past `protected` for both.
    #[test]
    fn git_ignored_secrets_are_not_offered_but_build_output_is() {
        let home = tempfile::tempdir().unwrap();
        let repo = home.path().join("code").join("proj");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::write(repo.join(".gitignore"), ".env\nbuild\nscratch\n").unwrap();

        // Each candidate has to clear the signal's size floor to be considered at all.
        let big = vec![0u8; (MIN_BYTES + 1) as usize];
        for dir in [".env", "build", "scratch"] {
            std::fs::create_dir(repo.join(dir)).unwrap();
            std::fs::write(repo.join(dir).join("payload.bin"), &big).unwrap();
        }
        // `scratch` is a folder nothing recognises that happens to hold a secret.
        std::fs::write(repo.join("scratch").join(".env"), b"TOKEN=shh").unwrap();

        let ctx = Ctx::new(Roots::for_test(home.path()), Rules::load(), Inventory::default());
        let offered: Vec<String> = find(&ctx, &Taken::default())
            .iter()
            .flat_map(|f| f.items.iter().map(|i| i.path.clone()))
            .collect();

        assert!(offered.iter().any(|p| p.ends_with("build")), "build output is what this signal is for; got {offered:?}");
        assert!(!offered.iter().any(|p| p.contains(".env")), "a git-ignored .env must never be offered; got {offered:?}");
        assert!(
            !offered.iter().any(|p| p.ends_with("scratch")),
            "an unrecognised folder holding a .env must not be offered either; got {offered:?}",
        );
    }
}
