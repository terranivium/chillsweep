use serde::Deserialize;

use crate::report::{Category, Tier};

// One knowledge base per platform. Splitting the files rather than tagging each rule is
// deliberate: `protected`, `system_names`, `generic_names` and `cache_names` are every bit as
// platform-shaped as the rules themselves, so a per-rule `platform` field would still leave
// those lists mixed together.
#[cfg(windows)]
const RULES_NAME: &str = "rules/windows.toml";
#[cfg(windows)]
const DEFAULT_RULES: &str = include_str!("../rules/windows.toml");
#[cfg(target_os = "macos")]
const RULES_NAME: &str = "rules/macos.toml";
#[cfg(target_os = "macos")]
const DEFAULT_RULES: &str = include_str!("../rules/macos.toml");

#[derive(Debug, Deserialize)]
pub struct Rules {
    pub protected: Vec<String>,
    /// Absolute: refused at removal time no matter what found it or how sure it was.
    ///
    /// `protected` is a discovery-time filter that a signal with positive, path-local proof is
    /// allowed to look past (see `Finding::vouched`). This list is the floor underneath that —
    /// credentials, cloud sync, sandboxed app data, ChillSweep's own files. Nothing passes.
    #[serde(default)]
    pub never_touch: Vec<String>,
    pub system_names: Vec<String>,
    pub generic_names: Vec<String>,
    pub steam_internal: Vec<String>,
    pub cache_names: Vec<String>,
    pub exe_stopwords: Vec<String>,
    pub build_dirs: Vec<String>,
    #[serde(default)]
    pub restore_dir: Vec<RestoreDir>,
    #[serde(default)]
    pub project_kind: Vec<ProjectKind>,
    #[serde(default)]
    pub rule: Vec<Rule>,
}

/// A kind of project folder, recognised by a marker file beside its content.
///
/// This is how the scan tells "a Pro Tools session" or "a Unity project" from any other folder,
/// which is the whole difficulty: the folder name says nothing, but `Session.ptx` or
/// `ProjectSettings/ProjectVersion.txt` says everything.
#[derive(Debug, Deserialize)]
pub struct ProjectKind {
    pub id: String,
    /// What to call it: "Pro Tools session", "Unity project".
    pub name: String,
    /// Names or `*` patterns that identify the folder. A marker may be a relative path
    /// (`ProjectSettings/ProjectVersion.txt`), written with `/` on both platforms.
    pub markers: Vec<String>,
    /// Parts of the project that the tool rebuilds by itself.
    #[serde(default)]
    pub regenerable: Vec<Regenerable>,
    /// Where this tool puts recorded or imported material, if it has a fixed place for it.
    ///
    /// Only kinds that declare this take part in the "created and never used" and "not opened in
    /// a year" roll-ups, because only then does the scan know what an empty one looks like. A
    /// LaTeX document or a `.blend` *is* the work, with no content folder to be missing — so for
    /// those this stays empty and the scan never claims they are unused.
    #[serde(default)]
    pub content: Vec<String>,
}

/// One rebuildable part of a project: a folder like `Library` or a file like `*.reapeaks`.
#[derive(Debug, Deserialize)]
pub struct Regenerable {
    pub name: String,
    pub tier: RuleTier,
    #[serde(default)]
    pub what: Option<String>,
    pub if_deleted: String,
}

#[derive(Debug, Deserialize)]
pub struct RestoreDir {
    pub name: String,
    pub how: String,
}

#[derive(Debug, Deserialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub paths: Vec<String>,
    pub tier: RuleTier,
    pub category: RuleCategory,
    pub what: String,
    pub if_deleted: String,
    /// Display-name fragments of programs that own this. If any is installed, a
    /// leftover rule stays quiet.
    #[serde(default)]
    pub owner: Vec<String>,
    /// Executable names (without .exe) that also prove the owner is present.
    #[serde(default)]
    pub owner_exe: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuleTier {
    Safe,
    Leftover,
    YourCall,
}

impl From<RuleTier> for Tier {
    fn from(t: RuleTier) -> Tier {
        match t {
            RuleTier::Safe => Tier::Safe,
            RuleTier::Leftover => Tier::Leftover,
            RuleTier::YourCall => Tier::YourCall,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleCategory {
    Cache,
    Leftover,
    Temp,
    Dev,
    Games,
}

impl From<RuleCategory> for Category {
    fn from(c: RuleCategory) -> Category {
        match c {
            RuleCategory::Cache => Category::Cache,
            RuleCategory::Leftover => Category::Leftover,
            RuleCategory::Temp => Category::Temp,
            RuleCategory::Dev => Category::Dev,
            RuleCategory::Games => Category::Games,
        }
    }
}

impl Rules {
    /// Parse a knowledge base. Separate from `load` so the tests can check every platform's
    /// file on whichever platform they happen to run on.
    pub fn parse(text: &str) -> Result<Rules, toml::de::Error> {
        toml::from_str(text)
    }

    /// The knowledge base compiled into this build.
    ///
    /// This panics rather than falling back to `Rules::default()` on purpose: an empty
    /// `protected` list would mean *nothing is protected*, which is far worse than not
    /// starting. The file is `include_str!`'d, so a failure here is a build-time mistake that
    /// the tests below catch.
    pub fn load() -> Rules {
        Rules::parse(DEFAULT_RULES).unwrap_or_else(|e| panic!("built-in {RULES_NAME} is invalid: {e}"))
    }

    pub fn is_system_name(&self, name: &str) -> bool {
        // On macOS almost everything under ~/Library is named by bundle id, and Apple's own
        // reverse-DNS prefix is unambiguous: `com.apple.*` is the OS's data, never a leftover,
        // whether or not the matching app is somewhere we thought to look. Listing them all by
        // hand would be both endless and unreliable.
        if !cfg!(windows) && (name.starts_with("com.apple.") || name == "com.apple") {
            return true;
        }
        self.system_names.iter().any(|n| n.eq_ignore_ascii_case(name))
    }

    pub fn is_generic_name(&self, name: &str) -> bool {
        self.generic_names.iter().any(|n| n.eq_ignore_ascii_case(name))
    }

    /// The project kind whose marker `dir` holds, if any.
    pub fn project_kind_of(&self, dir: &std::path::Path) -> Option<&ProjectKind> {
        self.project_kind.iter().find(|k| k.markers.iter().any(|m| marker_present(dir, m)))
    }
}

/// Is this marker present in `dir`?
///
/// A plain name with no `*` is a direct existence check, so `ProjectSettings/ProjectVersion.txt`
/// costs one `stat` rather than a directory listing. A pattern lists the folder and matches names.
fn marker_present(dir: &std::path::Path, marker: &str) -> bool {
    if !marker.contains('*') {
        // Markers are written with `/`; build the path a segment at a time so it works on both.
        return marker.split('/').fold(dir.to_path_buf(), |acc, seg| acc.join(seg)).exists();
    }
    crate::fsutil::children(dir).iter().any(|(p, md)| md.is_file() && crate::fsutil::wildcard(marker, &crate::fsutil::file_name(p)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tokens `Roots::expand` understands, per platform. Declared unconditionally so each
    /// file can be checked from either OS.
    const SHARED_TOKENS: [&str; 6] = ["%USERPROFILE%", "%CONFIG%", "%CACHE%", "%TEMP%", "%SHARED%", "%STEAM%"];
    const WINDOWS_TOKENS: [&str; 4] = ["%LOCALAPPDATA%", "%APPDATA%", "%LOCALLOW%", "%PROGRAMDATA%"];
    const MACOS_TOKENS: [&str; 5] = ["%LIBRARY%", "%PREFERENCES%", "%LOGS%", "%SAVEDSTATE%", "%CONTAINERS%"];

    /// Every knowledge base, checked on every platform.
    const ALL: [(&str, &str, &[&str]); 2] = [
        ("rules/windows.toml", include_str!("../rules/windows.toml"), &WINDOWS_TOKENS),
        ("rules/macos.toml", include_str!("../rules/macos.toml"), &MACOS_TOKENS),
    ];

    #[test]
    fn every_platforms_rules_parse() {
        for (name, text, _) in ALL {
            let r = Rules::parse(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(r.rule.len() > 20, "{name}: only {} rules", r.rule.len());
            assert!(r.rule.iter().all(|r| !r.paths.is_empty()), "{name}: a rule has no paths");
            assert!(!r.protected.is_empty(), "{name}: nothing is protected");
            let ids: std::collections::HashSet<_> = r.rule.iter().map(|r| &r.id).collect();
            assert_eq!(ids.len(), r.rule.len(), "{name}: rule ids must be unique");
        }
    }

    /// The one test that guards against destroying something irreplaceable.
    ///
    /// A project's regenerable list says "the tool rebuilds this, so it is safe to clear". Name
    /// a content folder there and the scan would offer up the recorded work itself. No amount of
    /// care in review substitutes for failing the build.
    #[test]
    fn no_project_offers_its_own_content_as_regenerable() {
        // The original recorded or imported material. Nothing rebuilds this, so it must never
        // appear in a regenerable list at any tier. Whole-name matches, so Pro Tools' `Fade
        // Files` and Cubase's `Images` are unaffected.
        const NEVER_LIST: [&str; 9] =
            ["audio files", "audio", "media", "recorded", "recordings", "samples", "assets", "content", "sources"];
        // Derived from the work, but quite possibly the thing someone was paid for. These may be
        // offered, but only ever as the user's call, never as safe.
        const NEVER_SAFE: [&str; 8] =
            ["bounced files", "bounces", "video files", "footage", "clips", "renders", "exports", "mixdowns"];

        for (name, text, _) in ALL {
            let r = Rules::parse(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            for kind in &r.project_kind {
                assert!(!kind.markers.is_empty(), "{name}: {} has no markers", kind.id);
                for part in &kind.regenerable {
                    let lower = part.name.to_lowercase();
                    assert!(
                        !NEVER_LIST.contains(&lower.as_str()),
                        "{name}: {} lists {:?} as regenerable, but that is where the work lives",
                        kind.id,
                        part.name
                    );
                    assert!(
                        !(NEVER_SAFE.contains(&lower.as_str()) && part.tier == RuleTier::Safe),
                        "{name}: {} calls {:?} safe, but it may be the finished deliverable — it must be your_call",
                        kind.id,
                        part.name
                    );
                    assert!(!part.if_deleted.trim().is_empty(), "{name}: {} / {:?} has no if_deleted text", kind.id, part.name);
                    assert!(
                        !kind.content.iter().any(|c| c.eq_ignore_ascii_case(&part.name)),
                        "{name}: {} says {:?} holds its content AND that it regenerates — it cannot be both",
                        kind.id,
                        part.name
                    );
                }
            }
            let ids: std::collections::HashSet<_> = r.project_kind.iter().map(|k| &k.id).collect();
            assert_eq!(ids.len(), r.project_kind.len(), "{name}: project kind ids must be unique");
        }
    }

    /// Both platforms must describe the same projects: a Unity project is a Unity project.
    #[test]
    fn both_platforms_know_the_same_projects() {
        let kinds = |text| {
            let r = Rules::parse(text).unwrap();
            let mut v: Vec<String> = r.project_kind.iter().map(|k| k.id.clone()).collect();
            v.sort();
            v
        };
        assert_eq!(kinds(ALL[0].1), kinds(ALL[1].1), "the two rules files have drifted apart");
    }

    /// A `%TOKEN%` the other platform owns expands to nothing, so the rule silently matches
    /// nothing — the easiest way to get a port wrong and never notice.
    #[test]
    fn rules_only_use_their_own_platforms_tokens() {
        for (name, text, own) in ALL {
            let allowed: Vec<&str> = SHARED_TOKENS.iter().chain(own.iter()).copied().collect();
            for token in text.split('%').skip(1).step_by(2) {
                let token = format!("%{token}%");
                // Skip anything that isn't token-shaped (prose can contain a stray `%`).
                if !token.chars().all(|c| c == '%' || c.is_ascii_uppercase() || c == '(' || c == ')' || c.is_ascii_digit()) {
                    continue;
                }
                assert!(allowed.contains(&token.as_str()), "{name} uses {token}, which it has no value for");
            }
        }
    }
}
