//! Launch agents, launch daemons and the Dock: what macOS has instead of `.lnk` shortcuts.
//!
//! A launch agent pointing at an app is proof the app is meant to be here, and the path it
//! points at is an install location. The Dock is the same idea: anything pinned there is an
//! app the user still has.

use std::path::{Path, PathBuf};

use super::{Inventory, Source};
use crate::fsutil;
use crate::roots::Roots;

const AGENT_DIRS: [&str; 3] = ["/Library/LaunchAgents", "/Library/LaunchDaemons", "/Library/StartupItems"];

pub fn gather(roots: &Roots, inv: &mut Inventory) {
    let mut dirs: Vec<PathBuf> = AGENT_DIRS.iter().map(PathBuf::from).collect();
    dirs.push(roots.home.join("Library").join("LaunchAgents"));
    for dir in dirs {
        for (file, md) in fsutil::children(&dir) {
            if md.is_file() && file.extension().is_some_and(|e| e.eq_ignore_ascii_case("plist")) {
                add_agent(&file, inv);
            }
        }
    }
    add_dock(roots, inv);
}

fn add_agent(path: &Path, inv: &mut Inventory) {
    let Some(dict) = read_plist(path) else { return };
    // `Label` is conventionally the owning app's bundle id.
    if let Some(label) = dict.get("Label").and_then(|v| v.as_string()) {
        inv.add_bundle_id(label, label.rsplit('.').next().unwrap_or(label));
    }
    // `Program`, or the first element of `ProgramArguments`, is what actually runs.
    let program = dict
        .get("Program")
        .and_then(|v| v.as_string())
        .map(str::to_string)
        .or_else(|| {
            dict.get("ProgramArguments")?
                .as_array()?
                .first()?
                .as_string()
                .map(str::to_string)
        });
    if let Some(program) = program {
        if program.starts_with('/') {
            inv.referenced.push(program.to_lowercase());
            inv.summary.shortcuts += 1;
        }
    }
}

/// Apps pinned to the Dock, from `persistent-apps[].tile-data.file-data._CFURLString`.
fn add_dock(roots: &Roots, inv: &mut Inventory) {
    let plist = roots.home.join("Library").join("Preferences").join("com.apple.dock.plist");
    let Some(dict) = read_plist(&plist) else { return };
    let Some(apps) = dict.get("persistent-apps").and_then(|v| v.as_array()) else { return };
    for app in apps {
        let Some(url) = app
            .as_dictionary()
            .and_then(|d| d.get("tile-data"))
            .and_then(|v| v.as_dictionary())
            .and_then(|d| d.get("file-data"))
            .and_then(|v| v.as_dictionary())
            .and_then(|d| d.get("_CFURLString"))
            .and_then(|v| v.as_string())
        else {
            continue;
        };
        // `file:///Applications/Safari.app/` → a path, percent-decoded and de-slashed.
        let path = url.strip_prefix("file://").unwrap_or(url).trim_end_matches('/');
        let path = percent_decode(path);
        let name = Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if let Some(stem) = name.strip_suffix(".app") {
            inv.add_name(stem, Source::Shortcut);
            inv.summary.shortcuts += 1;
        }
        inv.referenced.push(path.to_lowercase());
    }
}

/// `%20` → space. Dock URLs are percent-encoded and app names routinely contain spaces.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn read_plist(path: &Path) -> Option<plist::Dictionary> {
    let md = std::fs::metadata(path).ok()?;
    if !md.is_file() || md.len() > (1 << 20) {
        return None;
    }
    plist::Value::from_file(path).ok()?.into_dictionary()
}
