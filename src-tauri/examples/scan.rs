//! Command-line scan for development: `cargo run --example scan` prints a readable report, `-- --json`
//! the raw report. An example rather than a [[bin]] so the installer doesn't ship it.

use chillsweep_lib::report::Tier;
use chillsweep_lib::scan;

fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{bytes} B") } else { format!("{v:.1} {}", UNITS[u]) }
}

/// `--rules` prints how many paths each rule resolved to on this machine.
///
/// Most of the knowledge base describes software that isn't installed here, so a rule matching
/// nothing is normal and not a problem. What this catches is the opposite: a rule that *should*
/// have matched and didn't, which otherwise fails silently forever.
fn print_rule_coverage() {
    use chillsweep_lib::roots::Roots;
    use chillsweep_lib::rules::Rules;

    let roots = Roots::detect();
    let rules = Rules::load();
    let mut matched = 0;
    for rule in &rules.rule {
        let hits: usize = rule.paths.iter().map(|p| roots.resolve(p).len()).sum();
        if hits > 0 {
            matched += 1;
        }
        println!("{:>4}  {:<34} {}", hits, rule.id, if hits > 0 { "" } else { "(not on this machine)" });
    }
    println!("
{matched} of {} rules matched something here.", rules.rule.len());
    println!("{} project kinds known.", rules.project_kind.len());
    for p in &rules.protected {
        if roots.expand(p).is_none() {
            println!("  !! protected entry uses a variable this platform lacks: {p}");
        }
    }
}

fn main() {
    if std::env::args().any(|a| a == "--rules") {
        print_rule_coverage();
        return;
    }
    let report = scan::run();
    if std::env::args().any(|a| a == "--json") {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        return;
    }
    for total in &report.totals {
        let label = match total.tier {
            Tier::Safe => "SAFE TO CLEAR",
            Tier::Leftover => "LEFTOVERS",
            Tier::YourCall => "YOUR CALL",
        };
        println!("\n=== {label}: {} in {} findings", size(total.bytes), total.count);
        for f in report.findings.iter().filter(|f| f.tier == total.tier) {
            println!("{:>10}  {}  [{:?}]", size(f.bytes), f.title, f.confidence);
            for it in &f.items {
                println!("            {}", it.path);
            }
            for e in &f.evidence {
                println!("            - {e}");
            }
        }
    }
    // The sentence comes from the report, so this and the app can't drift apart or disagree
    // about what the counts are called on this platform.
    println!("\n{} in {:.1}s.", report.inventory.summary_text, report.duration_ms as f64 / 1000.0);
    for w in &report.warnings {
        println!("Note: {w}");
    }
}
