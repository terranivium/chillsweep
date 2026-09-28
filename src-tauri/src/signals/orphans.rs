//! App data folders that nothing installed on this PC owns.

use std::path::PathBuf;

use rayon::prelude::*;

use super::{age_days, dead_refs, finding, item, last_changed, looks_like_saves, Taken};
use crate::fsutil;
use crate::report::{Category, Confidence, Finding, Tier};
use crate::scan::Ctx;

const MIN_BYTES: u64 = 100 * 1024;

pub fn find(ctx: &Ctx, taken: &Taken) -> Vec<Finding> {
    let mut candidates: Vec<(PathBuf, &str)> = Vec::new();
    for root in [&ctx.roots.local, &ctx.roots.roaming] {
        candidates.extend(fsutil::child_dirs(root).into_iter().map(|d| (d, "app data")));
    }
    candidates.extend(fsutil::child_dirs(&ctx.roots.local.join("Programs")).into_iter().map(|d| (d, "install folder")));
    candidates.extend(
        fsutil::child_dirs(&ctx.roots.home)
            .into_iter()
            .filter(|d| fsutil::file_name(d).starts_with('.'))
            .map(|d| (d, "settings folder")),
    );
    candidates.retain(|(d, _)| {
        let name = fsutil::file_name(d);
        !ctx.rules.is_system_name(&name)
            && !ctx.rules.is_generic_name(&name)
            && !ctx.is_protected(d)
            && !ctx.is_owned(d)
            && !taken.covers(d)
    });

    candidates
        .par_iter()
        .filter_map(|(dir, kind)| {
            let name = fsutil::file_name(dir);
            if ctx.inv.owner_of(&name).is_some()
                || ctx.inv.exe_inside(dir).is_some()
                || ctx.inv.referenced_inside(dir).is_some()
            {
                return None;
            }
            let u = fsutil::usage(dir);
            // Empty folders are reported together by the empty-folder signal.
            if u.files == 0 || u.bytes < MIN_BYTES {
                return None;
            }
            let display = name.trim_start_matches('.');
            let mut f = finding(format!("orphan:{}", fsutil::lower(dir)), format!("{display} {kind}"), Tier::Leftover, Category::Leftover, Confidence::High);
            f.evidence.push(format!(
                "No installed program, program file, shortcut or running app matches “{display}”."
            ));
            let refs = dead_refs(dir);
            let has_refs = !refs.is_empty();
            f.evidence.extend(refs);

            let days = age_days(ctx, u.newest);
            if looks_like_saves(dir) {
                f.tier = Tier::YourCall;
                f.category = Category::Games;
                f.confidence = Confidence::Medium;
                f.what = Some("Looks like save data for a game that isn't installed.".into());
                f.if_deleted = Some("Your progress is gone for good unless the game keeps cloud saves. Keep it if you might play again.".into());
            } else if days < 1 {
                f.tier = Tier::Unknown;
                f.confidence = Confidence::Low;
                f.evidence.push("Something changed it in the last 24 hours, so a program may still be using it.".into());
            } else if days < 30 && !has_refs {
                f.confidence = Confidence::Medium;
                f.if_deleted = Some("Probably nothing, but it was used recently. If an app you still use loses its settings, restore it from the Recycle Bin.".into());
            } else {
                f.if_deleted = Some("Nothing, unless you reinstall the app and want its old settings back.".into());
            }
            f.evidence.push(last_changed(ctx, u.newest));
            f.last_modified = Some(u.newest);
            f.items.push(item(dir, &u));
            f.recompute_bytes();
            Some(f)
        })
        .collect()
}
