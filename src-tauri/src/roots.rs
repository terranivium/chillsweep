use std::path::{Path, PathBuf};

use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

use crate::fsutil;

/// Well-known folders the scan starts from.
#[derive(Debug, Clone)]
pub struct Roots {
    pub home: PathBuf,
    pub local: PathBuf,
    pub roaming: PathBuf,
    pub local_low: PathBuf,
    pub temp: PathBuf,
    pub program_data: PathBuf,
    pub program_files: Vec<PathBuf>,
    pub steam: Option<PathBuf>,
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).map(PathBuf::from).filter(|p| p.is_dir())
}

impl Roots {
    pub fn detect() -> Roots {
        let home = env_path("USERPROFILE").unwrap_or_else(|| PathBuf::from(r"C:\Users\Default"));
        let local = env_path("LOCALAPPDATA").unwrap_or_else(|| home.join(r"AppData\Local"));
        let roaming = env_path("APPDATA").unwrap_or_else(|| home.join(r"AppData\Roaming"));
        let local_low = home.join(r"AppData\LocalLow");
        // %TEMP% is sometimes an 8.3 short path; use the long form so paths compare cleanly.
        let temp = local.join("Temp");
        let program_data = env_path("ProgramData").unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
        let program_files = ["ProgramFiles", "ProgramFiles(x86)"]
            .iter()
            .filter_map(|k| env_path(k))
            .collect();
        Roots { home, local, roaming, local_low, temp, program_data, program_files, steam: steam_root() }
    }

    /// Expand %VARS% in a rules path. None if it uses a variable we don't have (e.g. no Steam).
    pub fn expand(&self, pattern: &str) -> Option<String> {
        let mut out = pattern.to_string();
        let vars: [(&str, Option<&Path>); 7] = [
            ("%USERPROFILE%", Some(&self.home)),
            ("%LOCALAPPDATA%", Some(&self.local)),
            ("%APPDATA%", Some(&self.roaming)),
            ("%LOCALLOW%", Some(&self.local_low)),
            ("%TEMP%", Some(&self.temp)),
            ("%PROGRAMDATA%", Some(&self.program_data)),
            ("%STEAM%", self.steam.as_deref()),
        ];
        for (var, val) in vars {
            if out.contains(var) {
                out = out.replace(var, &val?.to_string_lossy());
            }
        }
        Some(out)
    }

    /// Expand a rules path, resolving `*` wildcards segment by segment, and return what exists.
    pub fn resolve(&self, pattern: &str) -> Vec<PathBuf> {
        let Some(expanded) = self.expand(pattern) else { return Vec::new() };
        let mut segments = expanded.split('\\');
        let Some(first) = segments.next() else { return Vec::new() };
        let mut current = vec![PathBuf::from(format!("{first}\\"))];
        for seg in segments.filter(|s| !s.is_empty()) {
            let mut next = Vec::new();
            for base in &current {
                if seg.contains('*') {
                    for (child, _) in fsutil::children(base) {
                        if fsutil::wildcard(seg, &fsutil::file_name(&child)) {
                            next.push(child);
                        }
                    }
                } else {
                    let p = base.join(seg);
                    if p.exists() {
                        next.push(p);
                    }
                }
            }
            current = next;
            if current.is_empty() {
                break;
            }
        }
        current
    }
}

fn steam_root() -> Option<PathBuf> {
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(r"Software\Valve\Steam").ok()?;
    let raw: String = key.get_value("SteamPath").ok()?;
    let p = PathBuf::from(raw.replace('/', "\\"));
    p.is_dir().then_some(p)
}
