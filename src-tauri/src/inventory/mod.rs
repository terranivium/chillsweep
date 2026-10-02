//! What exists on this machine: installed programs, shortcuts, running processes,
//! program folders and executables, and Steam games. Gathered once per scan, read-only.

mod programs;
pub mod steam;

// The two "what is installed here?" sources are the only platform-specific part of the
// inventory. Each is a single `gather` call, so swapping them is all a port needs.
#[cfg(windows)]
mod registry;
#[cfg(windows)]
mod shortcuts;
#[cfg(target_os = "macos")]
mod bundles;
#[cfg(target_os = "macos")]
mod launchitems;

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
    /// The raw reverse-DNS bundle id, when this entry came from one. Kept unnormalised
    /// because the label boundaries are what make `com.google.Chrome.helper` match
    /// `com.google.Chrome`, and `norm` throws the dots away.
    pub id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    BundleId,
    /// A `/var/db/receipts` entry: proof something *was* installed from a package, which is not
    /// proof it still is. Good enough to keep the orphan signal quiet, never good enough to
    /// claim an app is present.
    InstallerReceipt,
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
            Source::BundleId => "installed app's identifier",
            Source::InstallerReceipt => "installer receipt",
            Source::InstalledProgram => "installed program",
            Source::Publisher => "publisher of an installed program",
            Source::ProgramFolder => "program folder",
            Source::Shortcut => {
                if cfg!(windows) {
                    "Start menu or desktop shortcut"
                } else {
                    "Dock item or login item"
                }
            }
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
    /// Lowercased paths something points at: shortcut or Dock targets, running programs, and
    /// install locations from the registry or from app bundles.
    pub referenced: Vec<String>,
    /// Lowercased paths of running executables.
    pub running: Vec<String>,
    pub steam: steam::SteamInfo,
    pub summary: InventorySummary,
}

pub fn normalize(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

/// Does this folder name look like a reverse-DNS bundle id rather than a display name?
///
/// The test is the first label: an identifier starts with a short alphanumeric token like
/// `com`, `org`, `io` or `net`. That separates `com.adobe.cep.CEPHtmlEngine Helper` (an id,
/// spaces and all) from `Cycling '74` (a company) and `.config` (a dotfolder).
fn looks_like_bundle_id(name: &str) -> bool {
    let mut labels = name.split('.');
    let Some(tld) = labels.next() else { return false };
    !tld.is_empty()
        && tld.len() <= 5
        && tld.chars().all(|c| c.is_ascii_alphanumeric())
        && labels.next().is_some_and(|next| !next.is_empty())
}

/// True if `outer` is `inner`'s identifier or one of its ancestors, cut at a label boundary.
/// `com.google.Chrome.helper` is within `com.google.Chrome`, but `com.googlex` is not.
fn id_within(inner: &str, outer: &str) -> bool {
    inner == outer || (inner.len() > outer.len() && inner.starts_with(outer) && inner.as_bytes()[outer.len()] == b'.')
}

/// The vendor part of a reverse-DNS id: `com.adobe` from `com.adobe.Photoshop`. None for a
/// two-label id, where the second label *is* the product and vendor matching would be far
/// too broad.
fn id_vendor(id: &str) -> Option<String> {
    let mut it = id.split('.');
    let tld = it.next()?;
    let org = it.next()?;
    it.next()?;
    (!tld.is_empty() && !org.is_empty()).then(|| format!("{tld}.{org}"))
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
        #[cfg(windows)]
        {
            registry::gather(&mut inv);
            shortcuts::gather(roots, &mut inv);
        }
        #[cfg(target_os = "macos")]
        {
            bundles::gather(roots, &mut inv);
            launchitems::gather(roots, &mut inv);
        }
        programs::gather_processes(&mut inv);
        programs::gather_program_folders(roots, rules, &mut inv);
        inv.steam = steam::gather(roots);
        let steam_names: Vec<String> =
            inv.steam.apps.iter().flat_map(|a| [a.name.clone(), a.installdir.clone()]).collect();
        for name in steam_names {
            inv.add_name(&name, Source::SteamGame);
        }
        inv.summary.steam_games = inv.steam.apps.len();
        inv.summary.summary_text = inv.summary.describe();
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
            self.names.push(NameEntry { norm, display: display.trim().to_string(), source, id: None });
        }
    }

    /// Record an installed app's reverse-DNS identifier, e.g. `com.google.Chrome`.
    pub fn add_bundle_id(&mut self, id: &str, display: &str) {
        self.add_identifier(id, display, Source::BundleId);
    }

    pub fn add_identifier(&mut self, id: &str, display: &str, source: Source) {
        let id = id.trim().to_lowercase();
        if id.split('.').count() < 2 {
            return;
        }
        self.names.push(NameEntry {
            norm: normalize(&id),
            display: display.trim().to_string(),
            source,
            id: Some(id),
        });
    }

    /// Does this name belong to something that is on the machine *right now*?
    ///
    /// Deliberately stricter than `owner_of`, which is happy with any hint of ownership because
    /// its job is to stay quiet. A caller that wants to say "this is App X's live cache" needs
    /// to know X is actually here, so an installer receipt does not count.
    pub fn present_owner_of(&self, name: &str) -> Option<&NameEntry> {
        // A reverse-DNS name has to be matched structurally here. The fuzzy rules in `owner_of`
        // will happily spot the word "install" inside `com.krotos.studio.mini-installer` and
        // call that the owner, which is how a finding ends up titled "Install cache". Those
        // rules still apply through `owner_of` itself, where a loose match only buys silence.
        let owner = if looks_like_bundle_id(name) { self.owner_of_bundle_id(name)? } else { self.owner_of(name)? };
        matches!(
            owner.source,
            Source::BundleId
                | Source::InstalledProgram
                | Source::StoreApp
                | Source::ProgramFolder
                | Source::Executable
                | Source::RunningProcess
                | Source::SteamGame
        )
        .then_some(owner)
    }

    /// The installed app a reverse-DNS folder name belongs to.
    ///
    /// Three levels, narrowest first. The looser two matter because an app's data is written
    /// by its helpers as well as itself: `com.google.Chrome.framework` and
    /// `com.adobe.cep.CEPHtmlEngine Helper` have no bundle of their own but are plainly owned.
    fn owner_of_bundle_id(&self, cand: &str) -> Option<&NameEntry> {
        let cand = cand.to_lowercase();
        let vendor = id_vendor(&cand);
        self.names
            .iter()
            .filter(|n| n.source == Source::BundleId)
            .find(|n| {
                let Some(id) = n.id.as_deref() else { return false };
                id_within(&cand, id)
                    || id_within(id, &cand)
                    || vendor.as_deref().is_some_and(|v| id_vendor(id).as_deref() == Some(v))
            })
    }

    /// Find something installed that a folder called `name` probably belongs to.
    pub fn owner_of(&self, name: &str) -> Option<&NameEntry> {
        // A reverse-DNS folder name is compared against installed bundle ids structurally.
        // The fuzzy rules below would get this wrong both ways: `normalize` strips the dots,
        // so `com.google.Chrome` becomes `comgooglechrome` and can never match the app's own
        // display name `Google Chrome`.
        if looks_like_bundle_id(name) {
            if let Some(owner) = self.owner_of_bundle_id(name) {
                return Some(owner);
            }
        }
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
            // Windows: `...\foo.exe`. macOS: `.../Contents/MacOS/Foo`, no extension.
            let want = if cfg!(windows) {
                format!("\\{}.exe", exe.to_lowercase())
            } else {
                format!("/{}", exe.to_lowercase())
            };
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

    fn with_ids(ids: &[(&str, &str)]) -> Inventory {
        let mut i = Inventory::default();
        for (id, display) in ids {
            i.add_bundle_id(id, display);
        }
        i
    }

    #[test]
    fn tells_bundle_ids_from_display_names() {
        assert!(looks_like_bundle_id("com.google.Chrome"));
        assert!(looks_like_bundle_id("com.adobe.cep.CEPHtmlEngine Helper (Renderer)"));
        assert!(looks_like_bundle_id("org.videolan.vlc"));
        // Not identifiers.
        assert!(!looks_like_bundle_id("Cycling '74"));
        assert!(!looks_like_bundle_id(".config"));
        assert!(!looks_like_bundle_id("Visual Studio Code"));
        assert!(!looks_like_bundle_id("Adobe"));
    }

    #[test]
    fn bundle_ids_own_their_helpers() {
        let i = with_ids(&[("com.google.Chrome", "Google Chrome"), ("com.spotify.client", "Spotify")]);
        // Exact, and a helper below it.
        assert!(i.owner_of("com.google.Chrome").is_some());
        assert!(i.owner_of("com.google.Chrome.framework").is_some());
        assert!(i.owner_of("com.spotify.client.helper").is_some());
        // The other direction: the installed id is more specific than the folder.
        assert!(i.owner_of("com.spotify").is_some());
        // A different app from the same vendor is owned; an unrelated vendor is not.
        assert!(i.owner_of("com.google.Keystone").is_some());
        assert!(i.owner_of("com.evil.thing").is_none());
        // Must not match on a shared prefix that isn't a label boundary.
        assert!(i.owner_of("com.googlex.other").is_none());
    }

    #[test]
    fn two_label_ids_do_not_match_by_vendor() {
        // `com.foo` has no vendor/product split, so vendor matching would own all of `com.*`.
        let i = with_ids(&[("com.foo", "Foo")]);
        assert!(i.owner_of("com.foo").is_some());
        assert!(i.owner_of("com.bar").is_none());
    }
}
