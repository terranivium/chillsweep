use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
use winreg::RegKey;

use super::{Inventory, Source};

const UNINSTALL_KEYS: [(winreg::HKEY, &str); 3] = [
    (HKEY_LOCAL_MACHINE, r"Software\Microsoft\Windows\CurrentVersion\Uninstall"),
    (HKEY_LOCAL_MACHINE, r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
    (HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Uninstall"),
];

/// Installed programs from the uninstall keys: names, publishers, install folders.
pub fn gather(inv: &mut Inventory) {
    for (hive, path) in UNINSTALL_KEYS {
        let Ok(root) = RegKey::predef(hive).open_subkey_with_flags(path, KEY_READ) else { continue };
        for sub in root.enum_keys().flatten() {
            let Ok(key) = root.open_subkey_with_flags(&sub, KEY_READ) else { continue };
            let Ok(name) = key.get_value::<String, _>("DisplayName") else { continue };
            inv.add_name(&name, Source::InstalledProgram);
            inv.summary.installed_programs += 1;
            if let Ok(publisher) = key.get_value::<String, _>("Publisher") {
                inv.add_name(&publisher, Source::Publisher);
            }
            for value in ["InstallLocation", "DisplayIcon"] {
                if let Ok(raw) = key.get_value::<String, _>(value) {
                    if let Some(p) = clean_path(&raw) {
                        if let Some(folder) = std::path::Path::new(&p)
                            .ancestors()
                            .find(|a| a.is_dir())
                            .and_then(|a| a.file_name())
                        {
                            inv.add_name(&folder.to_string_lossy(), Source::ProgramFolder);
                        }
                        inv.referenced.push(p.to_lowercase());
                    }
                }
            }
        }
    }
}

/// `"C:\Foo\bar.exe",0` → `C:\Foo\bar.exe`
fn clean_path(raw: &str) -> Option<String> {
    let s = raw.trim().trim_matches('"');
    let s = match s.rfind(',') {
        Some(i) if s[i + 1..].trim().trim_start_matches('-').chars().all(|c| c.is_ascii_digit()) => &s[..i],
        _ => s,
    };
    let s = s.trim().trim_matches('"').trim_end_matches('\\');
    let b = s.as_bytes();
    (b.len() > 3 && b[1] == b':' && b[2] == b'\\').then(|| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::clean_path;

    #[test]
    fn cleans_icon_paths() {
        assert_eq!(clean_path(r#""C:\Program Files\App\app.exe",0"#).as_deref(), Some(r"C:\Program Files\App\app.exe"));
        assert_eq!(clean_path(r"C:\Program Files\App\").as_deref(), Some(r"C:\Program Files\App"));
        assert_eq!(clean_path("MsiExec.exe /X{123}"), None);
    }
}
