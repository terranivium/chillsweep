use std::path::{Path, PathBuf};

use super::{Inventory, Source};
use crate::fsutil;
use crate::roots::Roots;

/// Start menu, desktop and taskbar shortcuts: their names and the paths they point at.
pub fn gather(roots: &Roots, inv: &mut Inventory) {
    let public = std::env::var_os("PUBLIC").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Users\Public"));
    let dirs = [
        roots.roaming.join(r"Microsoft\Windows\Start Menu\Programs"),
        roots.program_data.join(r"Microsoft\Windows\Start Menu\Programs"),
        roots.roaming.join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar"),
        roots.home.join("Desktop"),
        roots.home.join(r"OneDrive\Desktop"),
        public.join("Desktop"),
    ];
    for dir in dirs {
        fsutil::walk_dirs(&dir, 3, |d, _| {
            for (file, md) in fsutil::children(d) {
                if md.is_file() && file.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk")) {
                    add_shortcut(&file, inv);
                }
            }
            true
        });
    }
}

fn add_shortcut(lnk: &Path, inv: &mut Inventory) {
    inv.summary.shortcuts += 1;
    if let Some(stem) = lnk.file_stem() {
        inv.add_name(&stem.to_string_lossy(), Source::Shortcut);
    }
    let Ok(bytes) = std::fs::read(lnk) else { return };
    if bytes.len() > 1 << 20 {
        return;
    }
    for target in extract_paths(&bytes) {
        inv.referenced.push(target.to_lowercase());
    }
}

/// Pull absolute paths out of a .lnk file without a full parser: shortcut files store the
/// target as a NUL-terminated ANSI string and/or UTF-16 strings.
pub fn extract_paths(bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    // ANSI: X:\...\0
    let mut i = 0;
    while i + 3 < bytes.len() {
        if bytes[i].is_ascii_alphabetic() && bytes[i + 1] == b':' && bytes[i + 2] == b'\\' {
            let end = bytes[i..].iter().position(|&b| b == 0 || b < 0x20).map_or(bytes.len(), |p| i + p);
            if end - i > 3 {
                out.push(String::from_utf8_lossy(&bytes[i..end]).into_owned());
            }
            i = end;
        } else {
            i += 1;
        }
    }
    // UTF-16LE: X\0:\0\\\0 ... \0\0
    let mut i = 0;
    while i + 6 < bytes.len() {
        if bytes[i].is_ascii_alphabetic() && bytes[i + 1] == 0 && bytes[i + 2] == b':' && bytes[i + 3] == 0 && bytes[i + 4] == b'\\' && bytes[i + 5] == 0 {
            let mut units = Vec::new();
            let mut j = i;
            while j + 1 < bytes.len() {
                let u = u16::from_le_bytes([bytes[j], bytes[j + 1]]);
                if u < 0x20 {
                    break;
                }
                units.push(u);
                j += 2;
            }
            if units.len() > 3 {
                out.push(String::from_utf16_lossy(&units));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::extract_paths;

    #[test]
    fn finds_ansi_and_utf16_paths() {
        let mut bytes = b"junk\x00C:\\Games\\Foo\\foo.exe\x00more".to_vec();
        for u in "D:\\Tools\\bar.exe".encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0]);
        let paths = extract_paths(&bytes);
        assert!(paths.contains(&r"C:\Games\Foo\foo.exe".to_string()));
        assert!(paths.contains(&r"D:\Tools\bar.exe".to_string()));
    }
}
