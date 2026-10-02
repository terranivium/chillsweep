//! Installed apps, read from their `.app` bundles and from installer receipts.
//!
//! This is the macOS answer to the registry's uninstall keys, and it has two sources:
//!
//!   * **App bundles** — anything dragged to `/Applications` or installed from the App Store.
//!     `Contents/Info.plist` gives the display name, the bundle id and the executable name.
//!   * **Installer receipts** — `/var/db/receipts`, macOS' record of everything installed from
//!     a `.pkg`. Adobe, Office, audio plug-ins and drivers mostly arrive this way and leave no
//!     bundle in `/Applications` at all.
//!
//! A receipt proves something *was* installed, not that it still is. That is fine, and it is
//! how the Windows registry source is already used: everything here only ever makes the scan
//! quieter. Erring toward "something owns this" is the right default for a tool that deletes.

use std::path::{Path, PathBuf};

use super::{Inventory, Source};
use crate::fsutil;
use crate::roots::Roots;

/// Extra places apps live that aren't in `Roots::app_dirs`.
///
/// `/Applications/Utilities` is deliberately absent: the walk below already reaches it from
/// `/Applications`, and listing it again counted every app in it twice — which showed up as an
/// inflated "Checked against N installed apps" in the footer.
const EXTRA_APP_DIRS: [&str; 2] = ["/System/Applications", "/System/Library/CoreServices"];

/// Receipts for everything installed from a `.pkg`.
const RECEIPTS: &str = "/var/db/receipts";

/// How deep to look for bundles. Vendors nest them (`/Applications/Adobe Photoshop/Adobe
/// Photoshop.app`), and `walk_dirs` already refuses to descend into a bundle itself.
const APP_DEPTH: usize = 3;

pub fn gather(roots: &Roots, inv: &mut Inventory) {
    let mut dirs: Vec<PathBuf> = roots.app_dirs.clone();
    dirs.extend(EXTRA_APP_DIRS.iter().map(PathBuf::from));
    // Roots can nest or repeat, so dedupe the bundles rather than the directories: counting one
    // app twice would overstate how much the scan checked against.
    let mut bundles: Vec<PathBuf> = dirs.iter().flat_map(|d| find_bundles(d)).collect();
    bundles.sort();
    bundles.dedup();
    for bundle in &bundles {
        add_bundle(bundle, inv);
    }
    add_receipts(inv);
}

/// Every `.app` at or below `root`, without ever looking inside one.
fn find_bundles(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fsutil::walk_dirs(root, APP_DEPTH, |dir, _| {
        // `walk_dirs` never descends into a package, so bundles have to be picked up from the
        // listing of their parent rather than by being visited themselves.
        for child in fsutil::child_dirs(dir) {
            if child.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")) {
                out.push(child);
            }
        }
        true
    });
    out
}

fn add_bundle(bundle: &Path, inv: &mut Inventory) {
    let info = bundle.join("Contents").join("Info.plist");
    let dict = read_plist(&info);

    // The folder name is the one thing always available, and it is what the user sees.
    let folder = fsutil::file_name(bundle);
    let folder_name = folder.strip_suffix(".app").unwrap_or(&folder).to_string();

    let display = dict
        .as_ref()
        .and_then(|d| string_of(d, "CFBundleDisplayName").or_else(|| string_of(d, "CFBundleName")))
        .unwrap_or_else(|| folder_name.clone());
    let id = dict.as_ref().and_then(|d| string_of(d, "CFBundleIdentifier"));
    let exe = dict.as_ref().and_then(|d| string_of(d, "CFBundleExecutable"));

    // An App Store app carries a receipt inside its own bundle.
    let source = if bundle.join("Contents").join("_MASReceipt").join("receipt").is_file() {
        Source::StoreApp
    } else {
        Source::InstalledProgram
    };
    inv.add_name(&display, source);
    if folder_name != display {
        inv.add_name(&folder_name, source);
    }
    inv.summary.installed_programs += 1;

    if let Some(id) = id {
        inv.add_bundle_id(&id, &display);
    }
    if let Some(exe) = exe {
        let path = bundle.join("Contents").join("MacOS").join(&exe);
        inv.add_name(&exe, Source::Executable);
        inv.exe_paths.push(fsutil::lower(&path));
    }
    // The bundle itself is an install location, so nothing inside it is ever a leftover.
    inv.referenced.push(fsutil::lower(bundle));
}

/// Package ids from `/var/db/receipts`. The file name *is* the id, so no parsing is needed for
/// the common case — `com.adobe.acrobat.pro.pkg.plist` → `com.adobe.acrobat.pro`.
fn add_receipts(inv: &mut Inventory) {
    for (file, md) in fsutil::children(Path::new(RECEIPTS)) {
        if !md.is_file() {
            continue;
        }
        let name = fsutil::file_name(&file);
        let Some(id) = name.strip_suffix(".plist") else { continue };
        // Receipts are named `<id>.plist` and `<id>.bom`; some carry a `.pkg` infix.
        let id = id.strip_suffix(".pkg").unwrap_or(id);
        if id.split('.').count() >= 2 {
            // The last label is the best name a receipt offers, and it is often an internal one
            // (`LogicPro_AppStore`). It is only ever used to recognise ownership, never shown as
            // a title — see `Inventory::present_owner_of`.
            let display = id.rsplit('.').next().unwrap_or(id).to_string();
            inv.add_identifier(id, &display, Source::InstallerReceipt);
        }
    }
}

// ── plist reading ───────────────────────────────────────────────────────────────
// Deliberately the narrow `Value` API rather than Serde: there are thousands of Info.plist
// shapes in the wild and only three keys matter, so a struct would be brittle for no gain.
// Every step is fallible and every failure is silent — one unreadable plist must never stop
// a scan.

/// Info.plist files are small. Anything larger is not one, and is not worth parsing.
const MAX_PLIST: u64 = 1 << 20;

fn read_plist(path: &Path) -> Option<plist::Dictionary> {
    let md = std::fs::metadata(path).ok()?;
    if !md.is_file() || md.len() > MAX_PLIST {
        return None;
    }
    // Handles both XML and the binary `bplist00` format, which is what most bundles ship.
    plist::Value::from_file(path).ok()?.into_dictionary()
}

fn string_of(dict: &plist::Dictionary, key: &str) -> Option<String> {
    let s = dict.get(key)?.as_string()?.trim();
    (!s.is_empty()).then(|| s.to_string())
}
