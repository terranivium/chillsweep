use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

use crate::fsutil;
use crate::inventory::Inventory;
use crate::report::{Category, Finding, Report, SectionTotal, Tier, TierTotal};
use crate::roots::Roots;
use crate::rules::Rules;
use crate::signals;

/// Everything a signal needs to judge a path.
pub struct Ctx {
    pub roots: Roots,
    pub rules: Rules,
    pub inv: Inventory,
    pub now: u64,
    protected: Vec<String>,
    /// Lowercased paths nothing may ever remove, however certain it is.
    never_touch: Vec<String>,
    /// Paths a rule ties to a program that is still installed.
    owned: Vec<String>,
    /// Project folders found on this machine, each with the index of its kind in
    /// `rules.project_kind`. Shared so every signal can ask, including the ones that run
    /// before the projects signal.
    ///
    /// Found on first use, not in `new`: the search walks the user's folders, and
    /// `clean::guard_ctx` builds a `Ctx` purely to re-check protection and running processes
    /// immediately before removing something. Making that pay for a disk walk would put a
    /// visible pause in front of every clean.
    projects: OnceLock<Vec<(PathBuf, usize)>>,
    /// Unowned folders left out because something changed them in the last 24 hours.
    pub skipped_recent: AtomicUsize,
}

impl Ctx {
    pub fn new(roots: Roots, rules: Rules, inv: Inventory) -> Ctx {
        let protected = rules
            .protected
            .iter()
            .filter_map(|p| roots.expand(p))
            .map(|p| p.to_lowercase())
            .collect();
        let owned = rules
            .rule
            .iter()
            .filter(|r| (!r.owner.is_empty() || !r.owner_exe.is_empty()) && inv.find_installed(&r.owner, &r.owner_exe).is_some())
            .flat_map(|r| r.paths.iter().flat_map(|p| roots.resolve(p)))
            .map(|p| fsutil::lower(&p))
            .collect();
        let never_touch = rules.never_touch.iter().filter_map(|p| roots.expand(p)).map(|p| p.to_lowercase()).collect();
        Ctx {
            roots,
            rules,
            inv,
            now: fsutil::now(),
            protected,
            never_touch,
            owned,
            projects: OnceLock::new(),
            skipped_recent: AtomicUsize::new(0),
        }
    }

    /// Inside a folder that a rule says belongs to an installed program.
    pub fn is_owned(&self, p: &Path) -> bool {
        let l = fsutil::lower(p);
        self.owned.iter().any(|o| fsutil::is_within(&l, o))
    }

    /// Protected, inside something protected, or containing something protected.
    pub fn is_protected(&self, p: &Path) -> bool {
        let l = fsutil::lower(p);
        self.protected.iter().any(|x| fsutil::is_within(&l, x) || fsutil::is_within(x, &l))
    }

    /// Off limits for good. Unlike `is_protected`, no amount of evidence gets past this.
    pub fn never_touch(&self, p: &Path) -> bool {
        let l = fsutil::lower(p);
        self.never_touch.iter().any(|x| fsutil::is_within(&l, x) || fsutil::is_within(x, &l))
    }

    /// Why this location may not be removed, or None if it may be.
    ///
    /// The one place the rule lives, so the scan can hide findings that removal would refuse
    /// and `clean` can refuse them, without the two drifting apart.
    pub fn refuse_location(&self, p: &Path, vouched: bool) -> Option<&'static str> {
        if self.never_touch(p) {
            Some("This location is off limits.")
        } else if self.is_protected(p) && !vouched {
            Some("This location is protected.")
        } else {
            None
        }
    }

    pub fn may_remove(&self, p: &Path, vouched: bool) -> bool {
        self.refuse_location(p, vouched).is_none()
    }

    fn projects_list(&self) -> &[(PathBuf, usize)] {
        self.projects.get_or_init(|| crate::signals::find_projects(&self.roots, &self.rules))
    }

    /// The project folder `p` belongs to, with the kind of project it is.
    pub fn project_of(&self, p: &Path) -> Option<(&Path, &crate::rules::ProjectKind)> {
        let l = fsutil::lower(p);
        self.projects_list()
            .iter()
            .filter(|(root, _)| fsutil::is_within(&l, &fsutil::lower(root)))
            // The innermost match wins, so a project nested inside another is described as itself.
            .max_by_key(|(root, _)| root.as_os_str().len())
            .map(|(root, kind)| (root.as_path(), &self.rules.project_kind[*kind]))
    }

    /// Every project folder found, newest-first order not guaranteed.
    pub fn projects(&self) -> impl Iterator<Item = (&Path, &crate::rules::ProjectKind)> {
        self.projects_list().iter().map(|(root, kind)| (root.as_path(), &self.rules.project_kind[*kind]))
    }
}

pub fn run() -> Report {
    let started = Instant::now();
    let roots = Roots::detect();
    // Ask before anything else reads these folders. Done here it is one prompt at a time, in a
    // known order; left until after the scan, several rayon workers trip the prompts at once and
    // whatever the user denies has already been recorded as empty.
    let denied = crate::roots::denied_dirs(&roots.home);
    let rules = Rules::load();
    let inv = Inventory::gather(&roots, &rules);
    let ctx = Ctx::new(roots, rules, inv);

    let mut findings = signals::run_all(&ctx);
    // A finding whose items `clean` would refuse is a dead row in the UI: it looks actionable,
    // then silently skips. Drop those here rather than showing something that cannot work.
    findings.retain(|f| f.items.iter().all(|i| ctx.may_remove(Path::new(&i.path), f.vouched)));
    drop_nested(&mut findings);
    findings.sort_by(|a, b| a.tier.cmp(&b.tier).then(b.bytes.cmp(&a.bytes)));

    let (totals, projects) = totals(&findings);

    let mut warnings = Vec::new();
    // Say so when the OS is holding a door shut, rather than reporting nothing and letting the
    // user assume there was nothing there.
    for label in denied {
        warnings.push(format!(
            "macOS is blocking access to {label}, so it wasn't checked. Allow it in System Settings › Privacy & Security › Files and Folders, then scan again."
        ));
    }
    if ctx.roots.steam.is_none() {
        warnings.push("Steam wasn't found, so Steam libraries weren't checked.".into());
    }
    let recent = ctx.skipped_recent.load(Ordering::Relaxed);
    if recent > 0 {
        let folders = if recent == 1 { "1 unowned folder was".to_string() } else { format!("{recent} unowned folders were") };
        warnings.push(format!(
            "{folders} skipped because something changed them in the last 24 hours, so a program may still be using them. Scan again tomorrow."
        ));
    }
    Report { findings, totals, projects, inventory: ctx.inv.summary.clone(), duration_ms: started.elapsed().as_millis(), warnings }
}

/// Per-tier totals for the general clean-up, and the project findings' total kept apart from them.
fn totals(findings: &[Finding]) -> (Vec<TierTotal>, SectionTotal) {
    let general = || findings.iter().filter(|f| f.category != Category::Projects);
    let tiers = [Tier::Safe, Tier::Leftover, Tier::YourCall]
        .into_iter()
        .map(|tier| {
            let of_tier = general().filter(|f| f.tier == tier);
            TierTotal { tier, bytes: of_tier.clone().map(|f| f.bytes).sum(), count: of_tier.count() }
        })
        .collect();
    let projects = findings.iter().filter(|f| f.category == Category::Projects);
    (tiers, SectionTotal { bytes: projects.clone().map(|f| f.bytes).sum(), count: projects.count() })
}

/// If one finding's item sits inside another finding's item, keep only the outer one so
/// nothing is counted twice.
fn drop_nested(findings: &mut Vec<Finding>) {
    let all: Vec<(usize, String)> = findings
        .iter()
        .enumerate()
        .flat_map(|(i, f)| f.items.iter().map(move |it| (i, it.path.to_lowercase())))
        .collect();
    for (i, f) in findings.iter_mut().enumerate() {
        let before = f.items.len();
        f.items.retain(|it| {
            let p = it.path.to_lowercase();
            !all.iter().any(|(j, other)| *j != i && other != &p && fsutil::is_within(&p, other))
        });
        if f.items.len() != before {
            f.recompute_bytes();
        }
    }
    findings.retain(|f| !f.items.is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::Inventory;
    use crate::roots::Roots;
    use crate::rules::Rules;

    fn ctx() -> Ctx {
        Ctx::new(Roots::detect(), Rules::load(), Inventory::default())
    }

    /// Project findings get their own section, so they must not swell the tier totals.
    #[test]
    fn projects_are_totalled_apart_from_the_tiers() {
        let make = |tier, category, bytes| {
            let mut f = signals::finding("x", "x", tier, category, crate::report::Confidence::High);
            f.bytes = bytes;
            f
        };
        let findings = [
            make(Tier::Safe, Category::Cache, 10),
            make(Tier::Safe, Category::Projects, 100),
            make(Tier::YourCall, Category::Projects, 1000),
        ];
        let (tiers, projects) = totals(&findings);
        let safe = tiers.iter().find(|t| t.tier == Tier::Safe).unwrap();
        assert_eq!((safe.bytes, safe.count), (10, 1));
        assert_eq!(tiers.iter().find(|t| t.tier == Tier::YourCall).unwrap().count, 0);
        assert_eq!((projects.bytes, projects.count), (1100, 2));
    }

    /// `run`'s backstop drops findings that removal would refuse. That is right for a vague
    /// guess, and wrong for the signals that are *meant* to look inside protected folders —
    /// Downloads, git repos, game saves, and explicit rules. Those vouch, and this pins it:
    /// without the vouch the Downloads signal silently reports nothing at all.
    #[test]
    fn protected_folders_need_a_vouch_to_survive_the_backstop() {
        let ctx = ctx();
        let home = &ctx.roots.home;
        for dir in ["Downloads", "Documents", "Desktop"] {
            let path = home.join(dir).join("chillsweep-test-does-not-exist");
            assert!(!ctx.may_remove(&path, false), "{dir} is protected, so an unvouched finding must be dropped");
            assert!(ctx.may_remove(&path, true), "{dir} must be reachable by a signal that can prove what it found");
        }
    }
}
