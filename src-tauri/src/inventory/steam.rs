use std::path::{Path, PathBuf};

use regex::Regex;

use crate::roots::Roots;

#[derive(Debug, Clone)]
pub struct SteamApp {
    pub appid: String,
    pub name: String,
    pub installdir: String,
}

#[derive(Debug, Default)]
pub struct SteamInfo {
    /// `steamapps` folders of every Steam library.
    pub libraries: Vec<PathBuf>,
    pub apps: Vec<SteamApp>,
}

impl SteamInfo {
    pub fn is_installed_appid(&self, id: &str) -> bool {
        self.apps.iter().any(|a| a.appid == id)
    }

    pub fn is_installed_dir(&self, dir: &str) -> bool {
        self.apps.iter().any(|a| a.installdir.eq_ignore_ascii_case(dir))
    }
}

/// Values for `"key"  "value"` pairs in Valve's KeyValues (VDF/ACF) text.
pub fn vdf_values(text: &str, key: &str) -> Vec<String> {
    let re = Regex::new(&format!(r#"(?i)"{}"\s+"((?:[^"\\]|\\.)*)""#, regex::escape(key))).unwrap();
    re.captures_iter(text).map(|c| c[1].replace(r"\\", r"\")).collect()
}

pub fn gather(roots: &Roots) -> SteamInfo {
    let mut info = SteamInfo::default();
    let Some(steam) = &roots.steam else { return info };
    let mut libraries = vec![steam.join("steamapps")];
    if let Ok(text) = std::fs::read_to_string(steam.join(r"steamapps\libraryfolders.vdf")) {
        for p in vdf_values(&text, "path") {
            libraries.push(PathBuf::from(p).join("steamapps"));
        }
    }
    libraries.sort_by_key(|p| p.to_string_lossy().to_lowercase());
    libraries.dedup_by_key(|p| p.to_string_lossy().to_lowercase());
    libraries.retain(|p| p.is_dir());
    for lib in &libraries {
        info.apps.extend(read_manifests(lib));
    }
    info.libraries = libraries;
    info
}

fn read_manifests(steamapps: &Path) -> Vec<SteamApp> {
    let Ok(rd) = std::fs::read_dir(steamapps) else { return Vec::new() };
    rd.flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_lowercase();
            n.starts_with("appmanifest_") && n.ends_with(".acf")
        })
        .filter_map(|e| {
            let text = std::fs::read_to_string(e.path()).ok()?;
            Some(SteamApp {
                appid: vdf_values(&text, "appid").into_iter().next()?,
                name: vdf_values(&text, "name").into_iter().next().unwrap_or_default(),
                installdir: vdf_values(&text, "installdir").into_iter().next()?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::vdf_values;

    #[test]
    fn parses_vdf() {
        let text = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\t}\n}";
        assert_eq!(vdf_values(text, "path"), vec![r"C:\Program Files (x86)\Steam".to_string()]);
        let acf = "\"AppState\"\n{\n\t\"appid\"\t\t\"632360\"\n\t\"name\"\t\t\"Risk of Rain 2\"\n\t\"installdir\"\t\t\"Risk of Rain 2\"\n}";
        assert_eq!(vdf_values(acf, "appid"), vec!["632360"]);
        assert_eq!(vdf_values(acf, "installdir"), vec!["Risk of Rain 2"]);
    }
}
