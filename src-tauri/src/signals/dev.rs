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
            let (tier, what, if_deleted) = if let Some(r) = restore {
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
            let mut f = finding(format!("dev:{}", fsutil::lower(dir)), format!("{repo_name}\\{name}"), tier, Category::Dev, Confidence::High);
            f.what = Some(what);
            f.if_deleted = Some(if_deleted);
            f.evidence.push(format!("The {repo_name} repo's .gitignore excludes it, so it isn't part of the project's source."));
            f.evidence.push(last_changed(ctx, u.newest));
            f.last_modified = Some(u.newest);
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
            !n.starts_with('.') && n != "appdata" && n != "onedrive"
        })
        .collect();
    starts.push(ctx.roots.home.join(r"OneDrive\Documents"));
    let mut repos = Vec::new();
    for start in starts {
        fsutil::walk_dirs(&start, 3, |dir, depth| {
            let name = fsutil::file_name(dir).to_lowercase();
            if depth > 0 && NEVER_DESCEND.contains(&name.as_str()) {
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
