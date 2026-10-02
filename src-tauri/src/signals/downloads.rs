//! Big, old files in Downloads: videos, installers, disk images, archives.

use super::{age_days, finding, item, Taken};
use crate::fsutil;
use crate::report::{Category, Confidence, Finding, Tier};
use crate::scan::Ctx;

const MB: u64 = 1 << 20;

struct Kind {
    exts: &'static [&'static str],
    label: &'static str,
    min_bytes: u64,
    min_days: u64,
    if_deleted: &'static str,
}

const KINDS: [Kind; 6] = [
    Kind { exts: &["mkv", "mp4", "avi", "mov", "wmv", "m4v", "webm", "ts"], label: "Video", min_bytes: 200 * MB, min_days: 14, if_deleted: "Gone for good unless you have another copy." },
    Kind { exts: &["exe", "msi", "msix", "msixbundle", "appx", "dmg", "pkg", "mpkg"], label: "Installer", min_bytes: 5 * MB, min_days: 30, if_deleted: "Installers can usually be downloaded again from the publisher." },
    Kind { exts: &["iso", "img", "vhd", "vhdx"], label: "Disk image", min_bytes: 100 * MB, min_days: 30, if_deleted: "Gone for good unless you can download it again." },
    Kind { exts: &["zip", "7z", "rar", "tar", "gz", "xz"], label: "Archive", min_bytes: 100 * MB, min_days: 30, if_deleted: "Gone for good unless you extracted it or can download it again." },
    // An app is a folder, not a file, so it is matched separately — see `find` below.
    Kind { exts: &["app"], label: "Application", min_bytes: 20 * MB, min_days: 30, if_deleted: "Download the app again from its website, or move it to Applications if you still want it." },
    Kind { exts: &[], label: "Large file", min_bytes: 1024 * MB, min_days: 30, if_deleted: "Gone for good unless you have another copy." },
];

/// The catch-all, used when nothing else matches by extension.
const FALLBACK: usize = KINDS.len() - 1;

pub fn find(ctx: &Ctx, taken: &Taken) -> Vec<Finding> {
    let downloads = ctx.roots.home.join("Downloads");
    let mut out = Vec::new();
    for (file, md) in fsutil::children(&downloads) {
        // A downloaded macOS app is a `.app` bundle — a directory. Checking only for files, as
        // this did, means a 500 MB app left in Downloads is never mentioned. Other directories
        // are still skipped: a folder of loose files is the user's own arrangement.
        let is_app = md.is_dir() && fsutil::is_package(&file) && file.extension().is_some_and(|e| e.eq_ignore_ascii_case("app"));
        if (!md.is_file() && !is_app) || taken.covers(&file) {
            continue;
        }
        let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let kind = KINDS.iter().find(|k| k.exts.contains(&ext.as_str())).unwrap_or(&KINDS[FALLBACK]);
        let modified = fsutil::mtime(&md);
        let days = age_days(ctx, modified);
        if days < kind.min_days {
            continue;
        }
        // A bundle's own metadata says nothing about its size, so it has to be measured.
        let u = fsutil::usage(&file);
        if u.bytes < kind.min_bytes {
            continue;
        }
        let name = fsutil::file_name(&file);
        let mut f = finding(format!("download:{}", fsutil::lower(&file)), name, Tier::YourCall, Category::Downloads, Confidence::High);
        f.what = Some(format!("{} in your Downloads folder.", kind.label));
        f.if_deleted = Some(kind.if_deleted.into());
        f.evidence.push(format!("Downloaded {} and not changed since.", fsutil::when(ctx.now, modified)));
        if is_app {
            f.evidence.push("It's still in Downloads, so it was never moved to your Applications folder.".into());
        }
        f.last_modified = Some(modified);
        // Downloads is a protected folder, and this signal is meant to look in it — that is the
        // whole feature. The proof is specific and path-local: this exact file, of a known kind,
        // over a size floor, untouched for weeks. Without the vouch these findings are dropped
        // before the report is built and the signal reports nothing at all.
        f.vouched = true;
        f.items.push(item(&file, &u));
        f.recompute_bytes();
        out.push(f);
    }
    out
}
