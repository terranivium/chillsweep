//! What exists on this machine: installed programs, shortcuts, running processes,
//! program folders and executables, and Steam games. Gathered once per scan, read-only.

mod programs;
mod registry;
mod shortcuts;
pub mod steam;

use std::path::Path;

use crate::fsutil;
use crate::report::InventorySummary;
use crate::roots::Roots;
use crate::rules::Rules;

#[derive(Debug, Clone)]
pub struct NameEntry {
    /// Lowercase, letters and digits only.
    pub norm: String,
    /// Human-readable form for evidence sentences.
    pub display: String,
    pub source: Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    InstalledProgram,
    Publisher,
    ProgramFolder,
    Shortcut,
    Executable,
    RunningProcess,
    SteamGame,
    StoreApp,
}

impl Source {
    pub fn describe(self) -> &'static str {
        match self {
            Source::InstalledProgram => "installed program",
            Source::Publisher => "publisher of an installed program",
            Source::ProgramFolder => "program folder",
            Source::Shortcut => "Start menu or desktop shortcut",
            Source::Executable => "program file",
            Source::RunningProcess => "running program",
            Source::SteamGame => "installed Steam game",
            Source::StoreApp => "Microsoft Store app",
        }
    }
}

#[derive(Debug, Default)]
pub struct Inventory {
    pub names: Vec<NameEntry>,
    /// Lowercased full paths of executables found on disk.
    pub exe_paths: Vec<String>,
    /// Lowercased paths something points at: shortcut targets, running programs,
    /// install locations from the registry.
    pub referenced: Vec<String>,
    /// Lowercased paths of running executables.
    pub running: Vec<String>,
    pub steam: steam::SteamInfo,
    pub summary: InventorySummary,
}

pub fn normalize(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

/// First word of a multi-word name, if it is distinctive enough to identify a company.
fn first_word(s: &str) -> Option<String> {
    let mut words = s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty());
    let first = words.next()?.to_lowercase();
    (words.next().is_some() && first.len() >= 4).then_some(first)
}

impl Inventory {
    pub fn gather(roots: &Roots, rules: &Rules) -> Inventory {
        let mut inv = Inventory::default();
        registry::gather(&mut inv);
        shortcuts::gather(roots, &mut inv);
        programs::gather_processes(&mut inv);
        programs::gather_program_folders(roots, rules, &mut inv);
        inv.steam = steam::gather(roots);
        let steam_names: Vec<String> =
            inv.steam.apps.iter().flat_map(|a| [a.name.clone(), a.installdir.clone()]).collect();
        for name in steam_names {
            inv.add_name(&name, Source::SteamGame);
        }
        inv.summary.steam_games = inv.steam.apps.len();
        inv.names.sort_by(|a, b| a.norm.cmp(&b.norm));
        inv.names.dedup_by(|a, b| a.norm == b.norm);
        inv
    }

    /// Just the running programs: enough for the last-second checks before removing.
    pub fn running_only() -> Inventory {
        let mut inv = Inventory::default();
        programs::gather_processes(&mut inv);
        inv
    }

    pub fn add_name(&mut self, display: &str, source: Source) {
        let norm = normalize(display);
        if norm.len() >= 3 {
            self.names.push(NameEntry { norm, display: display.trim().to_string(), source });
        }
    }

    /// Find something installed that a folder called `name` probably belongs to.
    pub fn owner_of(&self, name: &str) -> Option<&NameEntry> {
        let cand = normalize(name);
        if cand.len() < 3 {
            return None;
        }
        let cand_company = first_word(name);
        self.names.iter().find(|n| {
            n.norm == cand
                // "Code" folder ↔ "Microsoft Visual Studio Code"
                || (cand.len() >= 4 && n.norm.contains(&cand))
                // "BraveSoftware" folder ↔ "Brave"
                || (n.norm.len() >= 5 && cand.contains(&n.norm))
                // "Sony Corporation" folder ↔ publisher "Sony Interactive Entertainment"
                || (n.source == Source::Publisher && cand_company.is_some() && first_word(&n.display) == cand_company)
        })
    }

    /// Is any of `owners` (display-name fragments) or `exes` installed or present?
    pub fn find_installed(&self, owners: &[String], exes: &[String]) -> Option<String> {
        for owner in owners {
            let o = normalize(owner);
            if let Some(n) = self
                .names
                .iter()
                .find(|n| matches!(n.source, Source::InstalledProgram | Source::StoreApp | Source::SteamGame) && n.norm.contains(&o))
            {
                return Some(n.display.clone());
            }
        }
        for exe in exes {
            let want = format!("\\{}.exe", exe.to_lowercase());
            if let Some(p) = self.exe_paths.iter().chain(&self.running).find(|p| p.ends_with(&want)) {
                return Some(p.clone());
            }
        }
        None
    }

    /// A program file inside `dir`, meaning an app is installed there.
    pub fn exe_inside(&self, dir: &Path) -> Option<&str> {
        let d = fsutil::lower(dir);
        self.exe_paths.iter().find(|p| fsutil::is_within(p, &d)).map(|s| s.as_str())
    }

    /// A shortcut target, running program or registered install location inside `dir`.
    pub fn referenced_inside(&self, dir: &Path) -> Option<&str> {
        let d = fsutil::lower(dir);
        self.referenced.iter().find(|p| fsutil::is_within(p, &d)).map(|s| s.as_str())
    }

    pub fn running_inside(&self, dir: &Path) -> Option<&str> {
        let d = fsutil::lower(dir);
        self.running.iter().find(|p| fsutil::is_within(p, &d)).map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inv(names: &[(&str, Source)]) -> Inventory {
        let mut i = Inventory::default();
        for (n, s) in names {
            i.add_name(n, *s);
        }
        i
    }

    #[test]
    fn owner_matching() {
        let i = inv(&[
            ("Microsoft Visual Studio Code (User)", Source::InstalledProgram),
            ("Brave", Source::InstalledProgram),
            ("NVIDIA Corporation", Source::Publisher),
            ("Git", Source::InstalledProgram),
        ]);
        let sony = inv(&[("Sony Interactive Entertainment", Source::Publisher)]);
        assert!(sony.owner_of("Sony Corporation").is_some());
        // Company matching only uses publishers, and needs a multi-word name.
        let game = inv(&[("Sony Game", Source::SteamGame)]);
        assert!(game.owner_of("Sony Corporation").is_none());
        assert!(i.owner_of("Code").is_some());
        assert!(i.owner_of("BraveSoftware").is_some());
        assert!(i.owner_of("NVIDIA Corporation").is_some());
        assert!(i.owner_of("NVIDIA Profile Inspector").is_some());
        assert!(i.owner_of("Sony Corporation").is_none());
        assert!(i.owner_of("librewolf").is_none());
        assert!(i.owner_of(".conda").is_none());
        // Short names never match by containment ("git" is inside lots of words).
        assert!(i.owner_of("digital").is_none());
    }
}
