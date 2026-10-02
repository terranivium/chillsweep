use std::path::Path;

use sysinfo::{ProcessesToUpdate, System};

use super::{Inventory, Source};
use crate::fsutil;
use crate::roots::Roots;
use crate::rules::Rules;

/// Paths of running programs. Read from the process list only; nothing is started.
pub fn gather_processes(inv: &mut Inventory) {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    for process in sys.processes().values() {
        let Some(exe) = process.exe() else { continue };
        let lower = fsutil::lower(exe);
        if is_system_exe(&lower) {
            continue;
        }
        inv.summary.running_processes += 1;
        if let Some(stem) = exe.file_stem() {
            inv.add_name(&stem.to_string_lossy(), Source::RunningProcess);
        }
        inv.referenced.push(lower.clone());
        inv.running.push(lower);
    }
    inv.running.sort();
    inv.running.dedup();
}

/// Folder names under Program Files / per-user Programs, and every .exe found in the
/// places apps install to (including per-user installs inside AppData).
pub fn gather_program_folders(roots: &Roots, rules: &Rules, inv: &mut Inventory) {
    for app_dir in &roots.app_dirs {
        for dir in fsutil::child_dirs(app_dir) {
            inv.add_name(&app_folder_name(&dir), Source::ProgramFolder);
            inv.summary.program_files_seen += 1;
        }
    }
    if let Some(per_user) = roots.per_user_programs() {
        for dir in fsutil::child_dirs(&per_user) {
            // An empty folder here is itself a leftover, not proof of an install.
            if has_exe(&dir, 3) {
                inv.add_name(&fsutil::file_name(&dir), Source::ProgramFolder);
            }
        }
    }
    if let Some(packages) = roots.store_packages() {
        for dir in fsutil::child_dirs(&packages) {
            // "Microsoft.WindowsTerminal_8wekyb3d8bbwe" → "Microsoft.WindowsTerminal"
            let name = fsutil::file_name(&dir);
            let family = name.split('_').next().unwrap_or(&name);
            let app = family.rsplit('.').next().unwrap_or(family);
            inv.add_name(app, Source::StoreApp);
        }
    }
    for (root, depth) in roots.exe_scan_roots() {
        index_exes(&root, depth, rules, inv);
    }
    // Portable apps often live in a folder at the top of the home directory.
    for (dir, md) in fsutil::children(&roots.home) {
        let name = fsutil::file_name(&dir);
        if md.is_dir() && !name.starts_with('.') && !name.eq_ignore_ascii_case("AppData") {
            index_exes(&dir, 2, rules, inv);
        }
    }
    inv.exe_paths.sort();
    inv.exe_paths.dedup();
}

/// The OS's own binaries, which say nothing about what the user installed.
fn is_system_exe(lower: &str) -> bool {
    #[cfg(windows)]
    const PREFIXES: [&str; 1] = [r"c:\windows\"];
    #[cfg(target_os = "macos")]
    const PREFIXES: [&str; 5] = ["/system/", "/usr/", "/sbin/", "/bin/", "/library/apple/"];
    PREFIXES.iter().any(|p| lower.starts_with(p))
}

/// `Foo.app` → `Foo`. On Windows a program folder is already just the app's name.
fn app_folder_name(dir: &Path) -> String {
    let name = fsutil::file_name(dir);
    name.strip_suffix(".app").unwrap_or(&name).to_string()
}

const SKIP_DIRS: [&str; 7] = ["temp", "packages", "node_modules", ".git", "microsoft", "windowsapps", "__pycache__"];

fn index_exes(root: &Path, max_depth: usize, rules: &Rules, inv: &mut Inventory) {
    fsutil::walk_dirs(root, max_depth, |dir, depth| {
        let name = fsutil::file_name(dir).to_lowercase();
        if depth > 0 && SKIP_DIRS.contains(&name.as_str()) {
            return false;
        }
        for (file, md) in fsutil::children(dir) {
            if !md.is_file() || !file.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")) {
                continue;
            }
            let stem = file.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
            if rules.exe_stopwords.iter().any(|w| w.eq_ignore_ascii_case(&stem)) || stem.contains("unins") {
                continue;
            }
            inv.add_name(&stem, Source::Executable);
            inv.exe_paths.push(fsutil::lower(&file));
        }
        true
    });
}

fn has_exe(dir: &Path, max_depth: usize) -> bool {
    let mut found = false;
    fsutil::walk_dirs(dir, max_depth, |d, _| {
        if found {
            return false;
        }
        found = fsutil::children(d)
            .iter()
            .any(|(f, md)| md.is_file() && f.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")));
        !found
    });
    found
}
