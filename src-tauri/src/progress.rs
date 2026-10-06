//! Where a scan has got to, for the page to show.
//!
//! A scan has no total to count towards: it finds its candidates stage by stage, and most of the
//! cost is walking folder trees whose size is the thing being measured. So the bar is built from
//! fixed stages with rough weights, each filling smoothly as its own candidates are examined, and
//! the page is also handed the path being looked at right now — the part that makes a long scan
//! visibly alive rather than possibly wedged.
//!
//! Everything here is read-only bookkeeping: counters, a clock and a callback.

use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crate::report::Finding;

/// How often a path from inside a signal may reach the page.
///
/// The signals examine candidates on every core at once, so without a gate this would emit tens of
/// thousands of events. 60ms is about one per frame, which is as fast as the page can paint and
/// faster than anyone can read a changing line.
const PATH_THROTTLE_MS: u64 = 60;

/// One step of a scan, and roughly how much of the bar it owns.
///
/// The weights only have to be right relative to each other. They come from timing a real scan
/// (`cargo run --example scan -- --progress` prints each stage's duration), and they are rough by
/// nature — how long a stage takes depends on what is actually on the machine. Being wrong makes
/// the bar uneven, nothing worse.
pub struct Stage {
    pub key: &'static str,
    pub label: &'static str,
    pub weight: u32,
}

/// What the page is told. Field names go over the wire as written, like `Report`'s.
#[derive(Clone, serde::Serialize)]
pub struct ScanProgress {
    /// Stable key for the stage, for anything that wants to match on it.
    pub stage: &'static str,
    /// What to show: "Looking through developer caches".
    pub label: &'static str,
    /// 1-based, for "3 of 16".
    pub step: usize,
    pub steps: usize,
    /// 0-100, and never smaller than the last one sent.
    pub percent: f32,
    /// The path being examined, when this update has one.
    pub path: Option<String>,
    /// Findings so far, so the page can fill in its totals before the scan ends.
    pub found: usize,
    pub bytes: u64,
}

type Sink = Arc<dyn Fn(&ScanProgress) + Send + Sync>;

pub struct Reporter {
    /// `None` is the silent reporter: every method returns before doing any work, so the
    /// command-line scan and the tests pay nothing.
    sink: Option<Sink>,
    stages: Vec<Stage>,
    total_weight: u32,
    base: Instant,
    /// Index into `stages`. Only moved between stages, which is single-threaded.
    index: AtomicUsize,
    units_total: AtomicUsize,
    units_done: AtomicUsize,
    /// Millis since `base` at the last emit.
    last_emit_ms: AtomicU64,
    /// Last percentage sent, in hundredths. Emits from parallel workers do not arrive in order,
    /// so the bar is clamped to never move backwards.
    percent_bp: AtomicU32,
    found: AtomicUsize,
    bytes: AtomicU64,
}

impl Reporter {
    /// Reports nothing. Used by `scan::run`, `clean::guard_ctx` and the tests.
    pub fn silent() -> Reporter {
        Reporter::build(None, Vec::new())
    }

    pub fn new(stages: Vec<Stage>, sink: Sink) -> Reporter {
        Reporter::build(Some(sink), stages)
    }

    fn build(sink: Option<Sink>, stages: Vec<Stage>) -> Reporter {
        let total_weight = stages.iter().map(|s| s.weight).sum();
        Reporter {
            sink,
            stages,
            total_weight,
            base: Instant::now(),
            index: AtomicUsize::new(0),
            units_total: AtomicUsize::new(0),
            units_done: AtomicUsize::new(0),
            last_emit_ms: AtomicU64::new(0),
            percent_bp: AtomicU32::new(0),
            found: AtomicUsize::new(0),
            bytes: AtomicU64::new(0),
        }
    }

    fn elapsed_ms(&self) -> u64 {
        self.base.elapsed().as_millis() as u64
    }

    /// Move to a named stage. Always reported, because losing a stage change to the throttle would
    /// leave the page showing the wrong thing until the next path happened along.
    pub fn stage(&self, key: &'static str) {
        if self.sink.is_none() {
            return;
        }
        let Some(i) = self.stages.iter().position(|s| s.key == key) else { return };
        self.index.store(i, Ordering::Relaxed);
        self.units_total.store(0, Ordering::Relaxed);
        self.units_done.store(0, Ordering::Relaxed);
        self.emit(None);
    }

    /// How many candidates are about to be examined. Known before the work starts, because every
    /// signal collects its candidates before handing them to rayon — which is what lets a stage
    /// fill smoothly instead of jumping.
    ///
    /// Added, not set: a few signals do several passes over their candidates within one stage, and
    /// each pass is real work the stage has to get through.
    pub fn units(&self, n: usize) {
        if self.sink.is_none() {
            return;
        }
        self.units_total.fetch_add(n, Ordering::Relaxed);
        self.emit(None);
    }

    /// Looking at this path now. Counts towards the stage; reported at most every
    /// `PATH_THROTTLE_MS`.
    pub fn examining(&self, path: &Path) {
        if self.sink.is_none() {
            return;
        }
        self.units_done.fetch_add(1, Ordering::Relaxed);
        self.show(path);
    }

    /// Show a path without counting it towards the stage. For a walk whose bar is driven by
    /// something coarser — one tick per search root, say — where counting every folder visited
    /// would swamp the denominator.
    pub fn showing(&self, path: &Path) {
        self.show(path);
    }

    fn show(&self, path: &Path) {
        let Some(sink) = self.sink.as_ref() else { return };
        if !self.claim_window() {
            return;
        }
        let p = path.to_string_lossy().into_owned();
        sink(&self.snapshot(Some(p)));
    }

    /// True for the one caller that gets to report in this throttle window.
    ///
    /// The clock is checked before the path is ever formatted, which is the whole point: the
    /// signals run on every core and a scan can have hundreds of thousands of candidates.
    fn claim_window(&self) -> bool {
        let now = self.elapsed_ms();
        let last = self.last_emit_ms.load(Ordering::Relaxed);
        if now.saturating_sub(last) < PATH_THROTTLE_MS {
            return false;
        }
        self.last_emit_ms.compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed).is_ok()
    }

    /// Count a candidate that has no path worth showing.
    pub fn counted(&self) {
        if self.sink.is_none() {
            return;
        }
        self.units_done.fetch_add(1, Ordering::Relaxed);
    }

    /// What a signal found. Always reported: it is what the page's running totals are made of.
    pub fn tally(&self, findings: &[Finding]) {
        if self.sink.is_none() {
            return;
        }
        self.found.fetch_add(findings.len(), Ordering::Relaxed);
        self.bytes.fetch_add(findings.iter().map(|f| f.bytes).sum::<u64>(), Ordering::Relaxed);
        self.emit(None);
    }

    /// The scan is over. Sends a final 100%, so the bar never stops short of the end.
    pub fn done(&self) {
        if self.sink.is_none() {
            return;
        }
        self.percent_bp.store(10_000, Ordering::Relaxed);
        self.emit(None);
    }

    fn emit(&self, path: Option<String>) {
        let Some(sink) = self.sink.as_ref() else { return };
        self.last_emit_ms.store(self.elapsed_ms(), Ordering::Relaxed);
        sink(&self.snapshot(path));
    }

    fn snapshot(&self, path: Option<String>) -> ScanProgress {
        let i = self.index.load(Ordering::Relaxed);
        let stage = self.stages.get(i);
        ScanProgress {
            stage: stage.map_or("", |s| s.key),
            label: stage.map_or("", |s| s.label),
            step: i + 1,
            steps: self.stages.len(),
            percent: self.percent(i),
            path,
            found: self.found.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
        }
    }

    fn percent(&self, i: usize) -> f32 {
        if self.total_weight == 0 {
            return 0.0;
        }
        let before: u32 = self.stages.get(..i).unwrap_or(&[]).iter().map(|s| s.weight).sum();
        let weight = self.stages.get(i).map_or(0, |s| s.weight) as f32;
        let total = self.units_total.load(Ordering::Relaxed);
        let fraction = if total == 0 {
            0.0
        } else {
            (self.units_done.load(Ordering::Relaxed) as f32 / total as f32).min(1.0)
        };
        let pct = (before as f32 + fraction * weight) / self.total_weight as f32 * 100.0;
        let bp = (pct * 100.0).clamp(0.0, 10_000.0) as u32;
        self.percent_bp.fetch_max(bp, Ordering::Relaxed).max(bp) as f32 / 100.0
    }
}

/// The stages of a scan, in order: what `scan::run_with_progress` walks through.
///
/// The signals supply their own labels and weights so the two can't drift apart from the array
/// that actually runs them.
pub fn scan_stages() -> Vec<Stage> {
    let mut stages = vec![
        Stage { key: "permissions", label: "Checking which folders we may read", weight: 1 },
        Stage { key: "inventory", label: "Seeing what's installed", weight: 20 },
        Stage { key: "projects-warm", label: "Finding your project folders", weight: 60 },
    ];
    stages.extend(crate::signals::stages());
    stages.push(Stage { key: "tidy", label: "Tidying up the results", weight: 2 });
    stages
}
