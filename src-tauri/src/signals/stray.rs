//! Stray files at the top of the home folder: empty files and leftover temp files.

use regex::Regex;

use super::{finding, item, Taken};
use crate::fsutil;
use crate::report::{Category, Confidence, Finding, Tier};
use crate::scan::Ctx;

pub fn find(ctx: &Ctx, taken: &Taken) -> Vec<Finding> {
    let tmp_re = Regex::new(r"(?i)\.tmp(\.|$)|^~\$").unwrap();
    let mut f = finding("stray-files", "Stray files", Tier::Safe, Category::Stray, Confidence::High);
    f.what = Some("Empty or temporary files at the top of your user folder.".into());
    f.if_deleted = Some("Nothing. They hold no data.".into());
    for (file, md) in fsutil::children(&ctx.roots.home) {
        if !md.is_file() || fsutil::is_hidden_system(&md) || taken.covers(&file) {
            continue;
        }
        let name = fsutil::file_name(&file);
        let reason = if md.len() == 0 {
            format!("{name} is empty (0 bytes).")
        } else if tmp_re.is_match(&name) && fsutil::now().saturating_sub(fsutil::mtime(&md)) > fsutil::DAY {
            format!("{name} is a temporary file an app didn't clean up.")
        } else {
            continue;
        };
        f.evidence.push(reason);
        f.items.push(item(&file, &fsutil::usage(&file)));
    }
    if f.items.is_empty() {
        Vec::new()
    } else {
        f.recompute_bytes();
        vec![f]
    }
}
