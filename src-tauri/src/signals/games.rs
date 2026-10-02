//! Steam library leftovers and save data for games that are no longer installed.

use rayon::prelude::*;

use super::{finding, item, last_changed, looks_like_saves, Taken};
use crate::fsutil;
use crate::report::{Category, Confidence, Finding, Tier};
use crate::scan::Ctx;

/// Per-app Steam folders named by appid.
const APPID_DIRS: [(&str, &str); 3] = [
    ("shadercache", "shader cache"),
    (r"workshop\content", "Workshop downloads"),
    ("compatdata", "Proton data"),
];

pub fn find_steam(ctx: &Ctx, taken: &Taken) -> Vec<Finding> {
    let steam = &ctx.inv.steam;
    let mut out = Vec::new();
    let mut appid_data = finding("steam-uninstalled-app-data", "Steam data for uninstalled games", Tier::Leftover, Category::Games, Confidence::High);
    appid_data.what = Some("Shader caches and Workshop content Steam keeps per game, identified by the game's app number.".into());
    appid_data.if_deleted = Some("Nothing. Steam downloads it again if you reinstall the game.".into());

    for lib in &steam.libraries {
        for dir in fsutil::child_dirs(&lib.join("common")) {
            let name = fsutil::file_name(&dir);
            if steam.is_installed_dir(&name)
                || ctx.rules.steam_internal.iter().any(|s| s.eq_ignore_ascii_case(&name))
                || taken.covers(&dir)
            {
                continue;
            }
            let u = fsutil::usage(&dir);
            let mut f = finding(format!("steam-common:{}", fsutil::lower(&dir)), format!("{name} (Steam)"), Tier::Leftover, Category::Games, Confidence::High);
            f.what = Some("A game folder in your Steam library.".into());
            f.if_deleted = Some("Nothing. Reinstalling the game from Steam recreates it.".into());
            f.evidence.push("Steam has no install record for this folder, so the game was uninstalled and its folder left behind.".into());
            if u.files == 0 {
                f.evidence.push("The folder is empty.".into());
            }
            f.evidence.push(last_changed(ctx, u.newest));
            f.last_modified = Some(u.newest);
            f.items.push(item(&dir, &u));
            f.recompute_bytes();
            out.push(f);
        }
        for (sub, label) in APPID_DIRS {
            for dir in fsutil::child_dirs(&lib.join(sub)) {
                let id = fsutil::file_name(&dir);
                if !id.chars().all(|c| c.is_ascii_digit()) || steam.is_installed_appid(&id) || taken.covers(&dir) {
                    continue;
                }
                let u = fsutil::usage(&dir);
                appid_data.evidence.push(format!("Steam app {id} isn't installed, but its {label} is still here."));
                appid_data.last_modified = appid_data.last_modified.max(Some(u.newest));
                appid_data.items.push(item(&dir, &u));
            }
        }
    }
    if !appid_data.items.is_empty() {
        appid_data.recompute_bytes();
        out.push(appid_data);
    }
    out
}

pub fn find_saves(ctx: &Ctx, taken: &Taken) -> Vec<Finding> {
    let candidates: Vec<_> = ctx
        .roots
        .save_roots()
        .iter()
        .flat_map(|(root, label)| fsutil::child_dirs(root).into_iter().map(move |d| (d, *label)))
        .filter(|(d, _)| {
            let name = fsutil::file_name(d);
            !ctx.rules.is_system_name(&name) && !taken.covers(d)
        })
        .collect();

    candidates
        .par_iter()
        .filter_map(|(dir, label)| {
            let name = fsutil::file_name(dir);
            if ctx.inv.owner_of(&name).is_some() || ctx.inv.exe_inside(dir).is_some() {
                return None;
            }
            let u = fsutil::usage(dir);
            if u.files == 0 {
                return None;
            }
            let mut f = finding(format!("saves:{}", fsutil::lower(dir)), format!("{name} {label}"), Tier::YourCall, Category::Games, Confidence::Medium);
            f.what = Some("Save files or settings a game keeps outside its install folder.".into());
            f.if_deleted = Some("Your progress is gone for good unless the game keeps cloud saves. Keep it if you might play again.".into());
            f.evidence.push(format!("No installed game or program matches “{name}”."));
            f.evidence.push(last_changed(ctx, u.newest));
            f.last_modified = Some(u.newest);
            // Vouching is what allows removal inside a protected folder, so it has to be earned.
            // The filters above only establish that nothing installed claims this folder, which is
            // an absence of evidence rather than evidence. Finding actual save files by name and
            // extension is the positive proof, so the vouch waits on that.
            f.vouched = looks_like_saves(dir);
            f.items.push(item(dir, &u));
            f.recompute_bytes();
            Some(f)
        })
        .collect()
}
