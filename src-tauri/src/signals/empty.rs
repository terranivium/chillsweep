//! Folders with nothing inside.

use super::{finding, item, Taken};
use crate::fsutil;
use crate::report::{Category, Confidence, Finding, Tier};
use crate::scan::Ctx;

pub fn find(ctx: &Ctx, taken: &Taken) -> Vec<Finding> {
    let mut dirs = fsutil::child_dirs(&ctx.roots.home);
    // The per-app data tree is judged by other signals, through roots that know its shape.
    // Looking at it here would only ever ask whether `~/Library` itself is empty.
    let app_data_root = if cfg!(windows) { "AppData" } else { "Library" };
    dirs.retain(|d| !fsutil::file_name(d).eq_ignore_ascii_case(app_data_root));
    for root in ctx.roots.empty_roots() {
        dirs.extend(fsutil::child_dirs(&root).into_iter().filter(|d| !ctx.rules.is_system_name(&fsutil::file_name(d))));
    }
    // `.cache`-style shared folders and Windows' own `Programs\Common` are containers apps
    // expect to exist.
    dirs.retain(|d| {
        let name = fsutil::file_name(d);
        !ctx.rules.is_generic_name(&name) && !name.eq_ignore_ascii_case("Common") && !ctx.is_owned(d)
    });

    let mut f = finding("empty-folders", "Empty folders", Tier::Safe, Category::Stray, Confidence::High);
    f.what = Some("Folders with no files inside, usually left behind by uninstalled apps.".into());
    f.if_deleted = Some("Nothing. An app that still uses one recreates it when it runs.".into());
    for dir in dirs {
        if ctx.is_protected(&dir) || taken.covers(&dir) {
            continue;
        }
        let u = fsutil::usage(&dir);
        if u.files == 0 {
            f.items.push(item(&dir, &u));
        }
    }
    if f.items.is_empty() {
        return Vec::new();
    }
    let names: Vec<_> = f.items.iter().map(|i| fsutil::file_name(std::path::Path::new(&i.path))).collect();
    f.evidence.push(format!("{} folders with nothing in them: {}.", names.len(), names.join(", ")));
    vec![f]
}
