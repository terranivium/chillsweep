use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use crate::fsutil;
use crate::inventory::Inventory;
use crate::report::{Finding, Report, Tier, TierTotal};
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
    /// Paths a rule ties to a program that is still installed.
    owned: Vec<String>,
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
        Ctx { roots, rules, inv, now: fsutil::now(), protected, owned, skipped_recent: AtomicUsize::new(0) }
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
}

pub fn run() -> Report {
    let started = Instant::now();
    let roots = Roots::detect();
    let rules = Rules::load();
    let inv = Inventory::gather(&roots, &rules);
    let ctx = Ctx::new(roots, rules, inv);

    let mut findings = signals::run_all(&ctx);
    drop_nested(&mut findings);
    findings.sort_by(|a, b| a.tier.cmp(&b.tier).then(b.bytes.cmp(&a.bytes)));

    let totals = [Tier::Safe, Tier::Leftover, Tier::YourCall]
        .into_iter()
        .map(|tier| {
            let of_tier = findings.iter().filter(|f| f.tier == tier);
            TierTotal { tier, bytes: of_tier.clone().map(|f| f.bytes).sum(), count: of_tier.count() }
        })
        .collect();

    let mut warnings = Vec::new();
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
    Report { findings, totals, inventory: ctx.inv.summary.clone(), duration_ms: started.elapsed().as_millis(), warnings }
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
