//! Command-line scan: `chillsweep-scan` prints a readable report, `chillsweep-scan --json` the raw report.

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

fn main() {
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
    let inv = &report.inventory;
    println!(
        "\nChecked {} installed programs, {} shortcuts, {} running programs, {} Program Files folders, {} Steam games in {:.1}s.",
        inv.installed_programs, inv.shortcuts, inv.running_processes, inv.program_files_seen, inv.steam_games,
        report.duration_ms as f64 / 1000.0
    );
    for w in &report.warnings {
        println!("Note: {w}");
    }
}
