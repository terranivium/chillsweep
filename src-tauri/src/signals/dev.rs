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
        .collect();

    candidates
        .par_iter()
        .filter_map(|(repo, dir)| {
            let u = fsutil::usage(dir);
            if u.bytes < MIN_BYTES {
                return None;
            }
            let name = fsutil::file_name(dir);
            let repo_name = fsutil::file_name(repo);
            let restore = ctx.rules.restore_dir.iter().find(|r| r.name.eq_ignore_ascii_case(&name));
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
                (
                    Tier::YourCall,
                    "Local files the project keeps out of git.".to_string(),
                    "Can't be recreated from the repo. Check it isn't data you need (downloaded models, databases, settings).".to_string(),
                )
            };
            let mut f = finding(format!("dev:{}", fsutil::lower(dir)), format!("{repo_name}{}{name}", std::path::MAIN_SEPARATOR), tier, Category::Dev, Confidence::High);
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
    let exclude = repo.join(r".git\info\exclude");
    if exclude.is_file() {
        b.add(exclude);
    }
    b.build().ok()
}

#[cfg(test)]
mod tests {
    use super::load_gitignore;

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
}
