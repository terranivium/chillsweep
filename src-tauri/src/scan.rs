use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

use crate::fsutil;
use crate::inventory::Inventory;
use crate::progress::Reporter;
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
    /// Where the scan has got to. Silent unless `with_progress` put a real one here, so the
    /// command-line scan, the tests and `clean::guard_ctx` pay nothing for it.
    pub progress: Reporter,
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
            progress: Reporter::silent(),
        }
    }

    /// Report this scan's progress. Only the scan does this: `clean::guard_ctx` has nobody to
    /// report to, and the tests and the command-line scan keep the silent one.
    pub fn with_progress(mut self, progress: Reporter) -> Ctx {
        self.progress = progress;
        self
    }

    /// Find the project folders now, rather than leaving it to whichever signal asks first.
    ///
    /// Without this the walk happens inside `dev`'s `par_iter`, on whichever worker reaches
    /// `project_of` first, while every other worker waits on the `OnceLock` — so a parallel stage
    /// collapses to one thread for the length of a whole-disk walk, and the time is charged to the
    /// wrong stage. Doing it here keeps it off `Ctx::new`, which `clean::guard_ctx` needs to stay
    /// cheap.
    pub fn warm_projects(&self) {
        self.projects_list();
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

    /// Does any part of this path name something that holds credentials or local settings?
    ///
    /// Every segment, not just the last: `~/.env` must be refused, and so must anything offered
    /// from inside it. The path lists can't do this job — they are absolute and don't expand
    /// wildcards, and a `.env` lives at an arbitrary depth in an arbitrary project.
    pub fn holds_secrets(&self, p: &Path) -> bool {
        p.components().any(|c| self.rules.is_never_name(&c.as_os_str().to_string_lossy()))
    }

    /// Why this location may not be removed, or None if it may be.
    ///
    /// The one place the rule lives, so the scan can hide findings that removal would refuse
    /// and `clean` can refuse them, without the two drifting apart.
    pub fn refuse_location(&self, p: &Path, vouched: bool) -> Option<&'static str> {
        if self.never_touch(p) {
            Some("This location is off limits.")
        } else if self.holds_secrets(p) {
            Some("This may hold credentials or local settings, so it is off limits.")
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
        self.projects.get_or_init(|| crate::signals::find_projects(&self.roots, &self.rules, &self.progress))
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
    run_with_progress(Reporter::silent())
}

/// A scan that says where it has got to. `run` is this with a reporter that says nothing, which
/// keeps `examples/scan.rs` and the tests free of anything to do with the app.
pub fn run_with_progress(progress: Reporter) -> Report {
    let started = Instant::now();
    progress.stage("permissions");
    let roots = Roots::detect();
    // Ask before anything else reads these folders. Done here it is one prompt at a time, in a
    // known order; left until after the scan, several rayon workers trip the prompts at once and
    // whatever the user denies has already been recorded as empty.
    let denied = crate::roots::denied_dirs(&roots.home);
    progress.stage("inventory");
    let rules = Rules::load();
    let inv = Inventory::gather(&roots, &rules);
    let ctx = Ctx::new(roots, rules, inv).with_progress(progress);

    ctx.progress.stage("projects-warm");
    ctx.warm_projects();

    let mut findings = signals::run_all(&ctx);
    ctx.progress.stage("tidy");
    // A finding whose items `clean` would refuse is a dead row in the UI: it looks actionable,
    // then silently skips. Drop those here rather than showing something that cannot work.
    findings.retain(|f| f.items.iter().all(|i| ctx.may_remove(Path::new(&i.path), f.vouched)));
    drop_nested(&mut findings);
    findings.sort_by(|a, b| a.tier.cmp(&b.tier).then(b.bytes.cmp(&a.bytes)));

    let (totals, projects) = totals(&findings);

    // Only things the user could act on, and only where silence would be misleading. Not having
    // a given program installed is the normal case, not a warning: a scan that announced every
    // place it had nothing to look at would bury the two below that actually matter.
    let mut warnings = Vec::new();
    // Say so when the OS is holding a door shut, rather than reporting nothing and letting the
    // user assume there was nothing there.
    for label in denied {
        warnings.push(format!(
            "macOS is blocking access to {label}, so it wasn't checked. Allow it in System Settings › Privacy & Security › Files and Folders, then scan again."
        ));
    }
    let recent = ctx.skipped_recent.load(Ordering::Relaxed);
    if recent > 0 {
        let folders = if recent == 1 { "1 unowned folder was".to_string() } else { format!("{recent} unowned folders were") };
        warnings.push(format!(
            "{folders} skipped because something changed them in the last 24 hours, so a program may still be using them. Scan again tomorrow."
        ));
    }
    ctx.progress.done();
    Report { findings, totals, projects, inventory: ctx.inv.summary.clone(), duration_ms: started.elapsed().as_millis(), warnings }
}

/// Per-tier totals for the general clean-up, and the project findings' total kept apart from them.
pub(crate) fn totals(findings: &[Finding]) -> (Vec<TierTotal>, SectionTotal) {
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

    /// A `.env` is a secret wherever it is, and the signal that reaches it is the one that vouches
    /// (a repo's own `.gitignore` names it, so `dev` looks past `protected`). That makes the name
    /// list the only thing standing in front of it, so this pins both halves: refused even with a
    /// vouch, and matched by pattern rather than exact name.
    #[test]
    fn credentials_are_refused_however_sure_the_signal_is() {
        let ctx = ctx();
        let repo = ctx.roots.home.join("code").join("chillsweep-test-repo");
        for name in [".env", ".env.local", ".env.production", "credentials.json", "id_rsa", "server.pem", "terraform.tfstate"] {
            let path = repo.join(name);
            assert_eq!(
                ctx.refuse_location(&path, true),
                Some("This may hold credentials or local settings, so it is off limits."),
                "{name} must be off limits even to a signal that can prove what it found",
            );
        }
        // Anything inside a secret-named folder is just as off limits.
        assert!(!ctx.may_remove(&repo.join(".env").join("lib"), true), "inside a .env is still a .env");
    }

    /// The guard has to stay narrow: these are exactly what the dev signal is for, and a list that
    /// swallowed them would quietly blunt the whole feature.
    #[test]
    fn the_credentials_guard_leaves_build_output_alone() {
        let ctx = ctx();
        let repo = ctx.roots.home.join("code").join("chillsweep-test-repo");
        for name in [".venv", "venv", "node_modules", "target", "build", "dist", "DerivedData", "env"] {
            assert!(
                !ctx.holds_secrets(&repo.join(name)),
                "{name} is ordinary build output or dependencies, not a secret",
            );
        }
    }
}
