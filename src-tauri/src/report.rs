use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Rebuilds itself or is pure junk.
    Safe,
    /// Belongs to something that is no longer installed.
    Leftover,
    /// Removable, but the user should decide (saves, big downloads, profiles).
    YourCall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Cache,
    Leftover,
    Temp,
    Stray,
    Dev,
    Games,
    Downloads,
    /// A project folder made by a creative tool or engine: a Pro Tools session, a Unity project.
    Projects,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

/// One file or folder covered by a finding.
#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub path: String,
    pub bytes: u64,
    pub files: u64,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// Stable id: rule id or signal name + path.
    pub id: String,
    pub title: String,
    pub tier: Tier,
    pub category: Category,
    pub confidence: Confidence,
    /// What the thing is, when we know.
    pub what: Option<String>,
    /// What happens if it's deleted, when we know.
    pub if_deleted: Option<String>,
    /// Plain-language reasons, one sentence each.
    pub evidence: Vec<String>,
    pub items: Vec<Item>,
    /// The signal had positive, path-local proof of what this is — a project marker file beside
    /// the cache it wants to clear — so removal may look past the broad `protected` list.
    /// Never past `never_touch`. Signals that are only guessing leave this false.
    pub vouched: bool,
    pub bytes: u64,
    /// Newest modification time anywhere inside, as unix seconds.
    pub last_modified: Option<u64>,
}

impl Finding {
    pub fn recompute_bytes(&mut self) {
        self.bytes = self.items.iter().map(|i| i.bytes).sum();
    }
}

/// Size and count of the findings kept apart from the tiers. See `Report::projects`.
#[derive(Debug, Clone, Copy, Serialize, Default)]
pub struct SectionTotal {
    pub bytes: u64,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TierTotal {
    pub tier: Tier,
    pub bytes: u64,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct InventorySummary {
    pub installed_programs: usize,
    pub shortcuts: usize,
    pub running_processes: usize,
    pub program_files_seen: usize,
    pub steam_games: usize,
    /// The footer sentence, written here rather than in the page.
    ///
    /// The counts above mean different things on each OS — "shortcuts" are Start menu links on
    /// Windows and Dock entries or launch agents on macOS — so whoever writes the sentence has
    /// to know which OS this is. Doing it in Rust keeps that knowledge out of the frontend and
    /// out of `examples/scan.rs`, which had its own copy of the same sentence.
    pub summary_text: String,
}

impl InventorySummary {
    /// "Checked against 231 installed apps, 31 Dock and login items, …".
    pub fn describe(&self) -> String {
        let n = |count: usize, one: &str, many: &str| format!("{count} {}", if count == 1 { one } else { many });
        // Each noun needs its own singular as well as its plural, or a machine with exactly one
        // of something reads in the other platform's vocabulary.
        let (app, apps) = if cfg!(windows) { ("installed program", "installed programs") } else { ("installed app", "installed apps") };
        let (link, links) = if cfg!(windows) { ("shortcut", "shortcuts") } else { ("Dock or login item", "Dock and login items") };
        let (folder, folders) =
            if cfg!(windows) { ("Program Files folder", "Program Files folders") } else { ("app folder", "app folders") };
        let mut parts = vec![
            n(self.installed_programs, app, apps),
            n(self.shortcuts, link, links),
            n(self.running_processes, "running program", "running programs"),
        ];
        if self.program_files_seen > 0 {
            parts.push(n(self.program_files_seen, folder, folders));
        }
        if self.steam_games > 0 {
            parts.push(n(self.steam_games, "Steam game", "Steam games"));
        }
        format!("Checked against {}", parts.join(", "))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub findings: Vec<Finding>,
    /// General clean-up only: project findings are left out of these.
    pub totals: Vec<TierTotal>,
    /// Project findings, which the page shows in their own section. They are specific to people who
    /// use those tools, so they stay out of the tier totals and the headline figure.
    pub projects: SectionTotal,
    pub inventory: InventorySummary,
    pub duration_ms: u128,
    pub warnings: Vec<String>,
}
