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
    pub bytes: u64,
    /// Newest modification time anywhere inside, as unix seconds.
    pub last_modified: Option<u64>,
}

impl Finding {
    pub fn recompute_bytes(&mut self) {
        self.bytes = self.items.iter().map(|i| i.bytes).sum();
    }
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
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub findings: Vec<Finding>,
    pub totals: Vec<TierTotal>,
    pub inventory: InventorySummary,
    pub duration_ms: u128,
    pub warnings: Vec<String>,
}
