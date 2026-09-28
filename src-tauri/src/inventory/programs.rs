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
        if lower.starts_with(r"c:\windows\") {
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
    for pf in &roots.program_files {
        for dir in fsutil::child_dirs(pf) {
            inv.add_name(&fsutil::file_name(&dir), Source::ProgramFolder);
            inv.summary.program_files_seen += 1;
        }
        index_exes(pf, 4, rules, inv);
    }
    let per_user = roots.local.join("Programs");
    for dir in fsutil::child_dirs(&per_user) {
        // An empty folder here is itself a leftover, not proof of an install.
        if has_exe(&dir, 3) {
            inv.add_name(&fsutil::file_name(&dir), Source::ProgramFolder);
        }
    }
    let packages = roots.local.join("Packages");
    for dir in fsutil::child_dirs(&packages) {
        // "Microsoft.WindowsTerminal_8wekyb3d8bbwe" → "Microsoft.WindowsTerminal"
        let name = fsutil::file_name(&dir);
        let family = name.split('_').next().unwrap_or(&name);
        let app = family.rsplit('.').next().unwrap_or(family);
        inv.add_name(app, Source::StoreApp);
    }
    for (root, depth) in [
        (roots.local.clone(), 4),
        (roots.roaming.clone(), 3),
        (roots.home.join(".local"), 3),
        (roots.home.join("Downloads"), 2),
        (roots.home.join("Desktop"), 2),
        (roots.home.join("Documents"), 2),
    ] {
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
