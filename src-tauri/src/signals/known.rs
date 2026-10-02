//! Curated rules from rules/default.toml.

use rayon::prelude::*;

use super::{dead_refs, finding, item, last_changed, Taken};
use crate::fsutil::{self, DAY};
use crate::report::{Confidence, Finding, Tier};
use crate::rules::RuleTier;
use crate::scan::Ctx;

const MIN_CACHE_BYTES: u64 = 1 << 20;

pub fn find(ctx: &Ctx, _taken: &Taken) -> Vec<Finding> {
    ctx.rules
        .rule
        .par_iter()
        .filter_map(|rule| {
            let paths: Vec<_> = rule
                .paths
                .iter()
                .flat_map(|p| ctx.roots.resolve(p))
                .filter(|p| ctx.inv.running_inside(p).is_none())
                .collect();
            if paths.is_empty() {
                return None;
            }
            let tier: Tier = rule.tier.into();
            let mut f = finding(rule.id.clone(), rule.name.clone(), tier, rule.category.into(), Confidence::High);
            f.what = Some(rule.what.clone());
            f.if_deleted = Some(rule.if_deleted.clone());

            let has_owner = !rule.owner.is_empty() || !rule.owner_exe.is_empty();
            if has_owner {
                if ctx.inv.find_installed(&rule.owner, &rule.owner_exe).is_some() {
                    // Owner is still here: caches stay useful to flag, leftovers are not leftovers.
                    if rule.tier != RuleTier::Safe {
                        return None;
                    }
                } else {
                    let owner = rule.owner.first().or(rule.owner_exe.first()).unwrap();
                    f.evidence.push(format!("{owner} isn't installed, and none of its program files were found."));
                }
            }

            let mut newest = 0;
            for p in &paths {
                let u = fsutil::usage(p);
                newest = newest.max(u.newest);
                // A curated rule names this exact path on purpose, which is the strongest proof
                // there is — stronger than any heuristic. That is what lets a rule reach inside a
                // protected folder, e.g. a browser's cache under its own profile directory.
                f.vouched = true;
                f.items.push(item(p, &u));
                if has_owner && rule.tier != RuleTier::Safe && u.is_dir {
                    f.evidence.extend(dead_refs(p));
                }
            }
            f.recompute_bytes();
            if tier == Tier::Safe && f.bytes < MIN_CACHE_BYTES {
                return None;
            }
            if tier != Tier::Safe && ctx.now.saturating_sub(newest) < DAY {
                f.evidence.push("Something changed it in the last 24 hours, so a program may still be using it.".into());
                f.confidence = Confidence::Medium;
            }
            f.evidence.push(last_changed(ctx, newest));
            f.last_modified = Some(newest);
            Some(f)
        })
        .collect()
}
