//! Where to start looking, on whichever OS this is.
//!
//! Every signal reads its starting folders from here, so this is the one file that knows the
//! difference between `%LOCALAPPDATA%` and `~/Library/Caches`. The fields are named for what
//! they mean rather than what either OS calls them, and the per-signal accessors at the bottom
//! keep the platform `cfg`s out of the signals themselves.

use std::path::{Path, PathBuf};

use crate::fsutil;

/// Well-known folders the scan starts from.
#[derive(Debug, Clone)]
pub struct Roots {
    pub home: PathBuf,
    /// Settings and data apps expect to keep. Windows: Roaming. macOS: Application Support.
    pub config: PathBuf,
    /// Data apps can afford to lose. Windows: Local. macOS: Caches.
    pub cache: PathBuf,
    /// Scratch folders. A list, because macOS has two worth looking at and Windows one.
    pub temp: Vec<PathBuf>,
    /// Machine-wide app data. Windows: ProgramData. macOS: `/Library`.
    pub shared: Option<PathBuf>,
    /// Where installed applications live.
    pub app_dirs: Vec<PathBuf>,
    pub steam: Option<PathBuf>,
    /// macOS only: `~/Library`. The rest of the per-app state hangs off this.
    library: Option<PathBuf>,
    /// Windows only: `AppData\LocalLow`, which has no macOS equivalent.
    local_low: Option<PathBuf>,
    /// Windows only: `Local\Programs`, where per-user installers put whole apps.
    per_user_programs: Option<PathBuf>,
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).map(PathBuf::from).filter(|p| p.is_dir())
}

#[cfg(target_os = "macos")]
fn existing(p: PathBuf) -> Option<PathBuf> {
    p.is_dir().then_some(p)
}

/// Top-level home folders that belong to the OS or an app, not to the user's own work.
fn is_system_home_dir(lower: &str) -> bool {
    const SHARED: [&str; 4] = ["appdata", "applications", "library", "public"];
    SHARED.contains(&lower)
}

/// Folders macOS gates behind a consent prompt, and what to call each one.
///
/// Reading any of these for the first time makes macOS ask the user. Everything else the scan
/// touches — `~/Library/Application Support`, `Caches`, `Preferences`, `Logs` — is ungated, and
/// the genuinely private places (Mail, Messages, Safari, Keychains) are in `protected` and never
/// read at all. So a TCC wall is a design invariant here rather than a problem to work around.
#[cfg(target_os = "macos")]
const GATED: [(&str, &str); 3] = [("Desktop", "your Desktop"), ("Documents", "your Documents folder"), ("Downloads", "your Downloads folder")];

/// Which gated folders this app is currently refused.
///
/// Worth doing because `fsutil::children` swallows the error: without this, a denial is
/// indistinguishable from an empty folder, and the scan quietly under-reports instead of saying
/// so. Run once, up front and single-threaded, so the prompts arrive one at a time rather than
/// from several rayon workers at once.
pub fn denied_dirs(home: &Path) -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        GATED
            .iter()
            .filter(|(dir, _)| matches!(std::fs::read_dir(home.join(dir)), Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied))
            .map(|(_, label)| (*label).to_string())
            .collect()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = home;
        Vec::new()
    }
}

/// Rewrite whichever separator a rules file used into this OS's.
fn normalize_separators(s: &str) -> String {
    let foreign = if cfg!(windows) { '/' } else { '\\' };
    s.replace(foreign, std::path::MAIN_SEPARATOR_STR)
}

impl Roots {
    #[cfg(windows)]
    pub fn detect() -> Roots {
        let home = env_path("USERPROFILE").unwrap_or_else(|| PathBuf::from(r"C:\Users\Default"));
        let cache = env_path("LOCALAPPDATA").unwrap_or_else(|| home.join(r"AppData\Local"));
        let config = env_path("APPDATA").unwrap_or_else(|| home.join(r"AppData\Roaming"));
        Roots {
            // %TEMP% is sometimes an 8.3 short path; use the long form so paths compare cleanly.
            temp: vec![cache.join("Temp")],
            shared: Some(env_path("ProgramData").unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))),
            app_dirs: ["ProgramFiles", "ProgramFiles(x86)"].iter().filter_map(|k| env_path(k)).collect(),
            local_low: Some(home.join(r"AppData\LocalLow")),
            per_user_programs: Some(cache.join("Programs")),
            library: None,
            steam: steam_root(),
            home,
            config,
            cache,
        }
    }

    #[cfg(target_os = "macos")]
    pub fn detect() -> Roots {
        let home = env_path("HOME").unwrap_or_else(|| PathBuf::from("/Users/Shared"));
        let library = home.join("Library");
        let config = library.join("Application Support");
        Roots {
            cache: library.join("Caches"),
            // $TMPDIR is a private per-user folder under /var/folders, and `/private/var/tmp`
            // survives reboots — both are ours and both collect installer leftovers.
            //
            // `/private/tmp` is deliberately absent. It is world-writable and shared, so it
            // holds other users' files as well as ours; the OS reaps it anyway; and removing
            // something there can break a build or a service that is running right now.
            temp: [env_path("TMPDIR"), existing(PathBuf::from("/private/var/tmp"))].into_iter().flatten().collect(),
            shared: existing(PathBuf::from("/Library")),
            app_dirs: [PathBuf::from("/Applications"), home.join("Applications")].into_iter().filter_map(existing).collect(),
            steam: existing(config.join("Steam")),
            local_low: None,
            per_user_programs: None,
            library: Some(library),
            home,
            config,
        }
    }

    /// A folder inside `~/Library`, or nothing on Windows.
    fn lib(&self, name: &str) -> Option<PathBuf> {
        self.library.as_ref().map(|l| l.join(name))
    }

    /// Expand `%VARS%` in a rules path. None if it uses a variable we don't have (e.g. no Steam).
    ///
    /// The tokens stay `%LIKE_THIS%` on both platforms: this is the rules file's own syntax, not
    /// a shell's, and writing a second parser for `$HOME` would buy nothing.
    pub fn expand(&self, pattern: &str) -> Option<String> {
        let mut out = pattern.to_string();
        let vars: [(&str, Option<PathBuf>); 15] = [
            ("%USERPROFILE%", Some(self.home.clone())),
            // Platform-neutral names, for the lines that mean the same thing in both files.
            ("%CONFIG%", Some(self.config.clone())),
            ("%CACHE%", Some(self.cache.clone())),
            ("%TEMP%", self.temp.first().cloned()),
            ("%SHARED%", self.shared.clone()),
            ("%STEAM%", self.steam.clone()),
            // Windows names.
            ("%LOCALAPPDATA%", Some(self.cache.clone())),
            ("%APPDATA%", Some(self.config.clone())),
            ("%LOCALLOW%", self.local_low.clone()),
            ("%PROGRAMDATA%", self.shared.clone()),
            // macOS names.
            ("%LIBRARY%", self.library.clone()),
            ("%PREFERENCES%", self.lib("Preferences")),
            ("%LOGS%", self.lib("Logs")),
            ("%SAVEDSTATE%", self.lib("Saved Application State")),
            ("%CONTAINERS%", self.lib("Containers")),
        ];
        for (var, val) in vars {
            if out.contains(var) {
                out = out.replace(var, &val?.to_string_lossy());
            }
        }
        // A rules file writes its own platform's separator, but the expanded string is also
        // compared against real paths (see `Ctx::new`), so settle on this OS's separator.
        Some(normalize_separators(&out))
    }

    /// Expand a rules path, resolving `*` wildcards segment by segment, and return what exists.
    pub fn resolve(&self, pattern: &str) -> Vec<PathBuf> {
        let Some(expanded) = self.expand(pattern) else { return Vec::new() };
        // Rules may use either separator: the Windows file writes `\`, the macOS file `/`.
        let mut segments = expanded.split(['\\', '/']).filter(|s| !s.is_empty());
        let mut current = if expanded.starts_with('/') {
            vec![PathBuf::from("/")]
        } else {
            // A drive-letter root: bare `C:` means "the current directory on C:", so it needs
            // a separator to mean the root itself.
            let Some(first) = segments.next() else { return Vec::new() };
            vec![PathBuf::from(format!("{first}{}", std::path::MAIN_SEPARATOR))]
        };
        for seg in segments {
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

    // ── Per-signal starting points ──────────────────────────────────────────────
    // Each of these answers one signal's question about where to look. Keeping them here
    // means a signal never needs to know which OS it is running on.

    /// Where this app keeps its own history log. Windows has always used Local, so it stays
    /// there; moving it would orphan every existing `history.jsonl`.
    pub fn app_data_dir(&self) -> PathBuf {
        if cfg!(windows) { self.cache.join("ChillSweep") } else { self.config.join("ChillSweep") }
    }

    /// Folders holding one subfolder per app, for the signals that ask "who owns this?".
    /// On macOS `Containers` and `Group Containers` are deliberately absent: sandboxed app
    /// data is both riskier to touch and listed as protected.
    pub fn app_data_roots(&self) -> Vec<PathBuf> {
        let mut v = vec![self.cache.clone(), self.config.clone()];
        v.extend(self.lib("Logs"));
        v.extend(self.lib("Saved Application State"));
        v
    }

    /// Candidates for the orphan signal, each with the word used to describe it.
    pub fn orphan_candidates(&self) -> Vec<(PathBuf, &'static str)> {
        let mut v: Vec<(PathBuf, &'static str)> = self.app_data_roots().into_iter().map(|d| (d, "app data")).collect();
        v.extend(self.per_user_programs.clone().map(|d| (d, "install folder")));
        v
    }

    /// Roots the version-leftover signal walks, with how deep to go in each.
    pub fn version_roots(&self) -> Vec<(PathBuf, usize)> {
        vec![(self.home.join(".local"), 4), (self.cache.clone(), 3), (self.config.clone(), 3)]
    }

    /// Roots the empty-folder signal checks.
    pub fn empty_roots(&self) -> Vec<PathBuf> {
        let mut v = vec![self.cache.clone(), self.config.clone()];
        v.extend(self.per_user_programs.clone());
        v
    }

    /// Places games keep saves, each with the word used to describe it.
    pub fn save_roots(&self) -> Vec<(PathBuf, &'static str)> {
        let mut v: Vec<(PathBuf, &'static str)> = Vec::new();
        if cfg!(windows) {
            v.push((self.home.join("Saved Games"), "saves"));
            v.push((self.home.join("Documents").join("My Games"), "saves"));
            v.extend(self.local_low.clone().map(|d| (d, "game data")));
        }
        // macOS has no equivalent and deliberately has none.
        //
        // A Mac game keeps its saves in `~/Library/Application Support/<Game>`, which the
        // orphan signal already walks — and `orphans` escalates to YourCall/Games itself when
        // `looks_like_saves` fires, with the same wording. Listing it here would only
        // duplicate that.
        //
        // The two tempting additions are both wrong. `~/Library/Containers` is sandboxed app
        // data, it is protected, and its `Data` folder is a symlink farm whose `Desktop` and
        // `Documents` point at the real ones. And `~/Documents` is the user's own space: this
        // signal looks past `protected` by design, so pointing it there means every project
        // folder and stray `Untitled` gets offered up as an uninstalled game's save data.
        v
    }

    /// Roots the executable index walks, with how deep to go in each.
    pub fn exe_scan_roots(&self) -> Vec<(PathBuf, usize)> {
        let mut v = vec![
            (self.cache.clone(), 4),
            (self.config.clone(), 3),
            (self.home.join(".local"), 3),
            (self.home.join("Downloads"), 2),
            (self.home.join("Desktop"), 2),
            (self.home.join("Documents"), 2),
        ];
        // Windows installs whole apps under Program Files; on macOS an app is a bundle, which
        // `bundles.rs` reads directly, so there is nothing for the executable index to find.
        if cfg!(windows) {
            v.extend(self.app_dirs.iter().map(|d| (d.clone(), 4)));
        }
        v
    }

    /// Windows' per-user install folder (`Local\Programs`). Nothing on macOS: an app there
    /// is a bundle in `/Applications` or `~/Applications`, which `app_dirs` already covers.
    pub fn per_user_programs(&self) -> Option<PathBuf> {
        self.per_user_programs.clone()
    }

    /// Windows' Store app folder. Nothing on macOS — App Store apps are ordinary bundles.
    pub fn store_packages(&self) -> Option<PathBuf> {
        self.per_user_programs.as_ref().map(|_| self.cache.join("Packages"))
    }

    /// Where to look for project folders.
    ///
    /// Deliberately wider than the dev signal's repo search: creative tools default to
    /// `~/Documents` and `~/Music`, which are protected and which `dev.rs` skips. A project
    /// marker file is proof enough to look (see `Finding::vouched`), so the cost of searching
    /// somewhere that turns out to hold nothing is one directory walk.
    pub fn project_search_roots(&self) -> Vec<PathBuf> {
        let mut v = vec![self.home.join("Documents"), self.home.join("Desktop"), self.home.join("Downloads")];
        if cfg!(windows) {
            v.push(self.home.join("Videos"));
            v.push(self.home.join("Music"));
            v.push(self.home.join("OneDrive").join("Documents"));
        } else {
            v.push(self.home.join("Movies"));
            v.push(self.home.join("Music"));
            v.push(self.home.join("Developer"));
        }
        // Plus the user's own top-level folders, where project trees usually really live.
        v.extend(
            fsutil::child_dirs(&self.home)
                .into_iter()
                .filter(|d| !fsutil::file_name(d).starts_with('.') && !is_system_home_dir(&fsutil::file_name(d).to_lowercase())),
        );
        v.sort();
        v.dedup();
        v
    }

    /// Extra folders the dev-clutter signal should look in beyond the top of home.
    pub fn dev_extra_starts(&self) -> Vec<PathBuf> {
        if cfg!(windows) {
            vec![self.home.join("OneDrive").join("Documents")]
        } else {
            vec![self.home.join("Developer")]
        }
    }
}

impl Roots {
    /// A `Roots` rooted entirely inside a temp directory, for tests. Every field points under
    /// `home`, so a test can lay out a fixture and know nothing else will be walked.
    #[cfg(test)]
    pub fn for_test(home: &std::path::Path) -> Roots {
        Roots {
            home: home.to_path_buf(),
            config: home.join("config"),
            cache: home.join("cache"),
            temp: vec![home.join("temp")],
            shared: None,
            app_dirs: Vec::new(),
            steam: None,
            library: None,
            local_low: None,
            per_user_programs: None,
        }
    }
}

#[cfg(windows)]
fn steam_root() -> Option<PathBuf> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(r"Software\Valve\Steam").ok()?;
    let raw: String = key.get_value("SteamPath").ok()?;
    let p = PathBuf::from(raw.replace('/', "\\"));
    p.is_dir().then_some(p)
}
