use serde::Deserialize;

use crate::report::{Category, Tier};

const DEFAULT_RULES: &str = include_str!("../rules/default.toml");

#[derive(Debug, Deserialize)]
pub struct Rules {
    pub protected: Vec<String>,
    pub system_names: Vec<String>,
    pub generic_names: Vec<String>,
    pub steam_internal: Vec<String>,
    pub cache_names: Vec<String>,
    pub exe_stopwords: Vec<String>,
    pub build_dirs: Vec<String>,
    #[serde(default)]
    pub restore_dir: Vec<RestoreDir>,
    #[serde(default)]
    pub rule: Vec<Rule>,
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
    pub fn load() -> Rules {
        toml::from_str(DEFAULT_RULES).expect("built-in rules/default.toml is invalid")
    }

    pub fn is_system_name(&self, name: &str) -> bool {
        self.system_names.iter().any(|n| n.eq_ignore_ascii_case(name))
    }

    pub fn is_generic_name(&self, name: &str) -> bool {
        self.generic_names.iter().any(|n| n.eq_ignore_ascii_case(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_rules_parse() {
        let r = Rules::load();
        assert!(r.rule.len() > 20);
        assert!(r.rule.iter().all(|r| !r.paths.is_empty()));
        let ids: std::collections::HashSet<_> = r.rule.iter().map(|r| &r.id).collect();
        assert_eq!(ids.len(), r.rule.len(), "rule ids must be unique");
    }
}
