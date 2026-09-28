//! Cache, log and update-download folders inside apps' data folders.

use std::path::PathBuf;

use rayon::prelude::*;

use super::{finding, item, last_changed, Taken};
use crate::fsutil;
use crate::report::{Category, Confidence, Finding, Tier};
use crate::scan::Ctx;

const MIN_BYTES: u64 = 20 << 20;

pub fn find(ctx: &Ctx, taken: &Taken) -> Vec<Finding> {
    let apps: Vec<PathBuf> = [&ctx.roots.local, &ctx.roots.roaming]
        .into_iter()
        .flat_map(|r| fsutil::child_dirs(r))
        .filter(|d| !ctx.rules.is_system_name(&fsutil::file_name(d)) && !ctx.is_protected(d) && !taken.covers(d))
        .collect();

    apps.par_iter()
        .filter_map(|app| {
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
