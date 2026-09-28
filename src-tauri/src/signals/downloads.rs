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

const KINDS: [Kind; 5] = [
    Kind { exts: &["mkv", "mp4", "avi", "mov", "wmv", "m4v", "webm", "ts"], label: "Video", min_bytes: 200 * MB, min_days: 14, if_deleted: "Gone for good unless you have another copy." },
    Kind { exts: &["exe", "msi", "msix", "msixbundle", "appx"], label: "Installer", min_bytes: 5 * MB, min_days: 30, if_deleted: "Installers can usually be downloaded again from the publisher." },
    Kind { exts: &["iso", "img", "vhd", "vhdx"], label: "Disk image", min_bytes: 100 * MB, min_days: 30, if_deleted: "Gone for good unless you can download it again." },
    Kind { exts: &["zip", "7z", "rar", "tar", "gz", "xz"], label: "Archive", min_bytes: 100 * MB, min_days: 30, if_deleted: "Gone for good unless you extracted it or can download it again." },
    Kind { exts: &[], label: "Large file", min_bytes: 1024 * MB, min_days: 30, if_deleted: "Gone for good unless you have another copy." },
];

pub fn find(ctx: &Ctx, taken: &Taken) -> Vec<Finding> {
    let downloads = ctx.roots.home.join("Downloads");
    let mut out = Vec::new();
    for (file, md) in fsutil::children(&downloads) {
        if !md.is_file() || taken.covers(&file) {
            continue;
        }
        let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let kind = KINDS.iter().find(|k| k.exts.contains(&ext.as_str())).unwrap_or(&KINDS[4]);
        let modified = fsutil::mtime(&md);
        let days = age_days(ctx, modified);
        if md.len() < kind.min_bytes || days < kind.min_days {
            continue;
        }
        let u = fsutil::usage(&file);
        let name = fsutil::file_name(&file);
        let mut f = finding(format!("download:{}", fsutil::lower(&file)), name, Tier::YourCall, Category::Downloads, Confidence::High);
        f.what = Some(format!("{} in your Downloads folder.", kind.label));
        f.if_deleted = Some(kind.if_deleted.into());
        f.evidence.push(format!("Downloaded {} and not changed since.", fsutil::when(ctx.now, modified)));
        f.last_modified = Some(modified);
        f.items.push(item(&file, &u));
        f.recompute_bytes();
        out.push(f);
    }
    out
}
