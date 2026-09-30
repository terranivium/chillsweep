# ChillSweep

A Windows cleanup app that finds what uninstalled apps, caches, dev projects and games left behind, and explains each item before you remove it.

**Download:** [ChillSweep-Setup.exe](https://github.com/terranivium/chillsweep/releases/latest/download/ChillSweep-Setup.exe) (Windows 10/11). Each [release](https://github.com/terranivium/chillsweep/releases) lists its SHA-256 and what's new. The installer isn't code-signed yet, so SmartScreen asks you to confirm on first run.

Most cleaners work from a fixed list of junk locations. ChillSweep checks your user folders against what is actually installed: the registry's installed programs and publishers, Start menu and desktop shortcuts, running programs, program files, Store apps and Steam libraries. That lets it spot things like:

- app data from programs you've uninstalled ("Docker Desktop isn't installed")
- config files pointing at folders that no longer exist ("environments.txt points to C:\…\miniconda3, which no longer exists")
- old versions an app kept after updating itself
- caches that rebuild themselves (shader caches, npm/pip/Cargo/Electron caches, app updater downloads)
- build output and dependencies that git ignores in your projects
- empty Steam game folders, Workshop data and shader caches for uninstalled games
- save data for games that aren't installed (flagged as "your call", never as junk)
- big old videos, installers and archives in Downloads

Every finding lands in a tier (**Safe to clear**, **Leftovers**, **Your call**) with plain-language evidence and what happens if you remove it. No AI model is involved at runtime: detection is general signals plus a small knowledge base (`src-tauri/rules/default.toml`).

## Safety

- Scanning only reads files and the registry. It never launches other programs.
- Protected locations (`.ssh`, Documents, browser profiles, password managers, …) are never flagged by the general signals.
- Nothing is selected by default, and removal needs a confirmation.
- Only items from the last scan can be removed, and each is re-checked right before removal (still exists, not protected, not in use by a running program).
- Items go to the Recycle Bin with an Undo button. Only "Safe to clear" items can optionally be deleted permanently.
- Every removal is logged to `%LOCALAPPDATA%\ChillSweep\history.jsonl`.
- The only thing ChillSweep fetches from the internet is its update check: on startup it reads `latest.json` from this repo's [releases](https://github.com/terranivium/chillsweep/releases). Updates are signed and only install when you choose to.

## Development

Requires Rust (MSVC toolchain), the Visual Studio C++ Build Tools, and Node.js 22+.

```sh
npm install
npm run tauri dev                                # run the app
cd src-tauri && cargo test --lib                 # unit tests
cd src-tauri && cargo run --bin chillsweep-scan  # command-line scan (add --json for raw output)
npm run notices                                  # regenerate src/third-party-notices.txt after dependency changes
npm run build                                    # Windows installer, versioned 1.{commit count}.0
npm run release                                  # build, sign and upload a draft release (see RELEASING.md)
```

User-facing changes get a bullet under `## Unreleased` in `CHANGELOG.md` as they're made; that becomes the release notes and the app's About → What's new.

Code layout:

- `src-tauri/src/inventory/`: what's installed (registry, shortcuts, processes, program files, Steam)
- `src-tauri/src/signals/`: one file per kind of clutter; each returns findings with evidence
- `src-tauri/src/clean.rs`: removal, undo and history
- `src-tauri/rules/default.toml`: protected paths and the knowledge base
- `src/`: the UI (plain HTML, CSS and JavaScript)
- `build-scripts/`: versioning, the release pipeline and the changelog bake

## License

ChillSweep is free software, licensed under the [GNU General Public License v3.0](LICENSE). The third-party components it bundles, and their licenses, are listed in [`src/third-party-notices.txt`](src/third-party-notices.txt).
