//! Curated rules from the platform's `rules/*.toml`.

use rayon::prelude::*;

use super::{dead_refs, finding, item, last_changed, Taken};
use crate::fsutil::{self, DAY};
use crate::report::{Confidence, Finding, Tier};
use crate::rules::RuleTier;
use crate::scan::Ctx;

const MIN_CACHE_BYTES: u64 = 1 << 20;

/// Naming an exact path in a curated rule is the strongest proof there is, so these findings may
/// reach inside a `protected` folder — browser caches live under the browser's own profile
/// directory, which is protected.
///
/// One constant because the per-path filter in `find` and `Finding::vouched` have to agree: a
/// filter that vouched less would drop those paths before the finding was built, and a rule whose
/// every path was dropped produces no row at all.
const VOUCHED: bool = true;

pub fn find(ctx: &Ctx, _taken: &Taken) -> Vec<Finding> {
    ctx.progress.units(ctx.rules.rule.len());
    ctx.rules
        .rule
        .par_iter()
        .filter_map(|rule| {
            let paths: Vec<_> = rule
                .paths
                .iter()
                .flat_map(|p| ctx.roots.resolve(p))
                .filter(|p| ctx.inv.running_inside(p).is_none())
                // Per path, because `scan::run`'s backstop keeps a finding only if *every* item
                // survives: one off-limits path would otherwise take the whole rule with it.
                //
                .filter(|p| ctx.may_remove(p, VOUCHED))
                .collect();
            let Some(first) = paths.first() else {
                ctx.progress.counted();
                return None;
            };
            ctx.progress.examining(first);
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
                f.vouched = VOUCHED;
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

#[cfg(test)]
mod tests {
    use super::VOUCHED;
    use crate::inventory::Inventory;
    use crate::roots::Roots;
    use crate::rules::Rules;
    use crate::scan::Ctx;

    /// Rules name paths inside protected folders on purpose, so both the per-path filter in `find`
    /// and the finding's own vouch have to say so. Filtering unvouched looks harmless and isn't:
    /// on Windows every `browser-caches` path lives under a protected profile folder, so the
    /// largest safe-tier row vanishes, and nothing says why.
    #[test]
    fn a_rule_may_name_a_path_inside_a_protected_folder() {
        let ctx = Ctx::new(Roots::detect(), Rules::load(), Inventory::default());
        let inside = ctx.roots.home.join("Documents").join("chillsweep-test-does-not-exist");
        assert!(!ctx.may_remove(&inside, false), "Documents is protected, so this is the case that matters");
        assert!(ctx.may_remove(&inside, VOUCHED), "a curated rule's path must survive the filter in `find`");
    }
}
