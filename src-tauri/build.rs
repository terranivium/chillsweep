use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    // Shown on the About screen. Build-time only: the app itself never launches programs.
    let hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "dev".into());
    let built = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    println!("cargo:rustc-env=LEFTOVER_GIT_HASH={hash}");
    println!("cargo:rustc-env=LEFTOVER_BUILD_TIME={built}");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/refs/heads");
    // The window and exe icons are embedded at compile time; rebuild when they're regenerated.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
