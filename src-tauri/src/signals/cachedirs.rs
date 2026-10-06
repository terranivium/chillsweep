//! Cache, log and update-download folders inside apps' data folders.

use std::path::PathBuf;

use rayon::prelude::*;

use super::{finding, item, last_changed, Taken};
use crate::fsutil;
use crate::report::{Category, Confidence, Finding, Tier};
use crate::scan::Ctx;

const MIN_BYTES: u64 = 20 << 20;

pub fn find(ctx: &Ctx, taken: &Taken) -> Vec<Finding> {
    let mut out = nested_caches(ctx, taken);
    out.extend(whole_cache_dirs(ctx, taken));
    out
}

/// Cache folders *inside* an app's data folder: `Discord/Cache`, `Foo/User Data/Code Cache`.
fn nested_caches(ctx: &Ctx, taken: &Taken) -> Vec<Finding> {
    let apps: Vec<PathBuf> = ctx
        .roots
        .app_data_roots()
        .iter()
        .flat_map(|r| fsutil::child_dirs(r))
        .filter(|d| !ctx.rules.is_system_name(&fsutil::file_name(d)) && ctx.may_remove(d, false) && !taken.covers(d))
        .collect();

    ctx.progress.units(apps.len());
    apps.par_iter()
        .filter_map(|app| {
            ctx.progress.examining(app);
            let app_name = fsutil::file_name(app);
            // Look one and two levels down: `Discord\Cache`, `Foo\User Data\Cache`.
            let mut hits = Vec::new();
            for level1 in std::iter::once(app.clone()).chain(fsutil::child_dirs(app)) {
                for name in &ctx.rules.cache_names {
                    let candidate = level1.join(name);
                    if candidate.is_dir() && !taken.covers(&candidate) && !hits.contains(&candidate) {
                        hits.push(candidate);
                    }
                }
            }
            if hits.is_empty() || ctx.inv.running_inside(app).is_some() {
                return None;
            }
            let mut f = finding(format!("cachedirs:{}", fsutil::lower(app)), format!("{app_name} caches"), Tier::Safe, Category::Cache, Confidence::Medium);
            f.what = Some(format!("Cache, log and update-download folders inside {app_name}'s data."));
            f.if_deleted = Some(format!("Rebuilt as {app_name} runs. Close {app_name} first."));
            let mut newest = 0;
            for h in &hits {
                let u = fsutil::usage(h);
                newest = newest.max(u.newest);
                f.items.push(item(h, &u));
            }
            f.recompute_bytes();
            if f.bytes < MIN_BYTES {
                return None;
            }
            let names: Vec<_> = hits.iter().map(|h| h.strip_prefix(app).unwrap_or(h).to_string_lossy().into_owned()).collect();
            f.evidence.push(format!("Folders named like caches: {}.", names.join(", ")));
            f.evidence.push(last_changed(ctx, newest));
            f.last_modified = Some(newest);
            Some(f)
        })
        .collect()
}

/// On macOS a direct child of `~/Library/Caches` *is itself* the cache — `~/Library/Caches/
/// com.spotify.client` is Spotify's, whole and entire, with no `Cache` subfolder to look for.
/// That has no Windows analogue, where `%LOCALAPPDATA%\Spotify` mixes cache in with settings.
///
/// This only reports caches whose owner **is** installed. An unowned one is a leftover, and the
/// orphan signal says something truer about it.
fn whole_cache_dirs(ctx: &Ctx, taken: &Taken) -> Vec<Finding> {
    if cfg!(windows) {
        return Vec::new();
    }
    let caches = fsutil::child_dirs(&ctx.roots.cache);
    ctx.progress.units(caches.len());
    caches
        .par_iter()
        .filter_map(|dir| {
            ctx.progress.examining(dir);
            let name = fsutil::file_name(dir);
            if ctx.rules.is_system_name(&name) || !ctx.may_remove(dir, false) || taken.covers(dir) || ctx.is_owned(dir) {
                return None;
            }
            // The owner has to be here *now*. An installer receipt would prove only that
            // something was once installed, which would have this claiming a long-gone app's
            // leftovers were its live cache. The orphan signal handles those, and truthfully.
            let owner = ctx.inv.present_owner_of(&name)?;
            let owner_name = owner.display.clone();
            let u = fsutil::usage(dir);
            if u.bytes < MIN_BYTES {
                return None;
            }
            let mut f = finding(
                format!("cache-dir:{}", fsutil::lower(dir)),
                format!("{owner_name} cache"),
                Tier::Safe,
                Category::Cache,
                Confidence::High,
            );
            f.what = Some(format!("Everything {owner_name} has cached. The whole folder is cache — macOS keeps it separate from settings."));
            f.if_deleted = Some(format!("Rebuilt as {owner_name} runs. Settings and logins are kept elsewhere and are not affected."));
            f.evidence.push(format!("It sits in your Caches folder and belongs to {owner_name}, which is installed."));
            f.evidence.push(last_changed(ctx, u.newest));
            f.last_modified = Some(u.newest);
            f.items.push(item(dir, &u));
            f.recompute_bytes();
            Some(f)
        })
        .collect()
}
