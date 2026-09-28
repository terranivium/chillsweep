//! Temp items nothing has touched for a week.

use rayon::prelude::*;

use super::{age_days, finding, item, Taken};
use crate::fsutil;
use crate::report::{Category, Confidence, Finding, Tier};
use crate::scan::Ctx;

const MIN_AGE_DAYS: u64 = 7;
const BIG: u64 = 50 << 20;
const IF_DELETED: &str = "Nothing. Installers and apps leave these behind when they don't clean up after themselves.";

pub fn find(ctx: &Ctx, taken: &Taken) -> Vec<Finding> {
    let entries: Vec<_> = fsutil::children(&ctx.roots.temp)
        .into_iter()
        .filter(|(p, _)| !taken.covers(p))
        .map(|(p, _)| p)
        .collect();
    let old: Vec<_> = entries
        .par_iter()
        .map(|p| (p, fsutil::usage(p)))
        .filter(|(_, u)| age_days(ctx, u.newest) >= MIN_AGE_DAYS)
        .collect();

    let mut out = Vec::new();
    let mut rest = finding("temp-old", "Old temp files", Tier::Safe, Category::Temp, Confidence::High);
    rest.what = Some("Temporary files in your user Temp folder.".into());
    rest.if_deleted = Some(IF_DELETED.into());
    for (path, u) in old {
        if u.bytes >= BIG {
            let name = fsutil::file_name(path);
            let mut f = finding(format!("temp:{}", fsutil::lower(path)), format!("Temp\\{name}"), Tier::Safe, Category::Temp, Confidence::High);
            f.what = Some("A leftover folder in your user Temp folder.".into());
            f.if_deleted = Some(IF_DELETED.into());
            f.evidence.push(format!("Nothing inside has changed in {} days.", age_days(ctx, u.newest)));
            if u.is_dir {
                let mut names: Vec<_> = fsutil::children(path).iter().map(|(c, _)| fsutil::file_name(c)).collect();
                names.sort();
                if !names.is_empty() {
                    let sample: Vec<_> = names.iter().take(3).cloned().collect();
                    f.evidence.push(format!("Contains {} items, e.g. {}.", names.len(), sample.join(", ")));
                }
            }
            f.last_modified = Some(u.newest);
            f.items.push(item(path, &u));
            f.recompute_bytes();
            out.push(f);
        } else {
            rest.last_modified = rest.last_modified.max(Some(u.newest));
            rest.items.push(item(path, &u));
        }
    }
    if !rest.items.is_empty() {
        rest.recompute_bytes();
        rest.evidence.push(format!("{} items nothing has touched in over {MIN_AGE_DAYS} days.", rest.items.len()));
        out.push(rest);
    }
    out
}
