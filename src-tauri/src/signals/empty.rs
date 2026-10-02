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
        if !ctx.may_remove(&dir, false) || taken.covers(&dir) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::Inventory;
    use crate::roots::Roots;
    use crate::rules::Rules;

    /// An empty `~/.aws` is on the absolute list. It must be left out on its own, not sink the
    /// whole finding when `scan::run` drops anything removal would refuse.
    #[test]
    fn an_off_limits_folder_is_skipped_not_fatal() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".aws")).unwrap();
        std::fs::create_dir(tmp.path().join("Leftover")).unwrap();
        let ctx = Ctx::new(Roots::for_test(tmp.path()), Rules::load(), Inventory::default());

        let found = find(&ctx, &Taken::default());
        assert_eq!(found.len(), 1);
        let paths: Vec<_> = found[0].items.iter().map(|i| fsutil::file_name(std::path::Path::new(&i.path))).collect();
        assert_eq!(paths, ["Leftover"]);
        assert!(found[0].items.iter().all(|i| ctx.may_remove(std::path::Path::new(&i.path), found[0].vouched)));
    }
}
