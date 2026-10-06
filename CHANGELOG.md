# Changelog

**This file is the source of truth for every release's notes, past and present.** Never type notes into
the GitHub UI: edit them here and push, so the two can't drift apart.

`build-scripts/release-notes.mjs` lifts the **`## Unreleased`** section into the draft's "What's new"
during `npm run release`, and every build bakes this file into the app's About → What's new. Write
bullets here as you make the change, in words a user would understand, rather than reconstructing them
at release time.

| Task | Command |
| --- | --- |
| Close out a shipped version | `npm run changelog:promote` |
| Fix the release currently in flight | `npm run release-notes -- --refresh` |
| Fix an **already-published** release | `npm run release-notes -- --notes=1.42.0` |

Promotion happens **after** publishing, not before: the version is `1.{git rev-list --count HEAD}.0`, so
the commit that renamed a heading would itself bump the count and make that heading wrong by one.

## Unreleased

- **ChillSweep will never offer anything that holds credentials or local settings**, wherever it
  finds it. A `.env` and its variants, SSH and signing keys, `.netrc`, `.npmrc`, credentials and
  service-account files, keystores and Terraform state are now off limits by name, at any depth, in
  any folder — including inside a code repository, where a project's own `.gitignore` used to be
  treated as permission enough. A git-ignored folder nothing recognises is also left alone if one of
  those is sitting directly inside it.
- **ChillSweep now checks that dependencies really can be reinstalled before offering to remove
  them.** A `node_modules`, `.venv`, `venv`, `vendor`, `Pods` or `Carthage` folder is only offered
  when the thing that rebuilds it is still beside it — a `package.json`, a `requirements.txt` or
  `pyproject.toml`, a lockfile, a `Podfile`. Without one, that folder is the only copy of what's in
  it, and "reinstall the dependencies" was advice that would have failed.
- A git-ignored folder called `env` is no longer assumed to be a Python virtual environment. It's
  still offered, but described as local files that can't be recreated from the repo, rather than as
  dependencies you can reinstall.
- The Trash itself is no longer offered for removal. An empty Trash used to turn up in the list of
  empty folders, which is never what anyone means — and it's where ChillSweep puts what it removes.
- ChillSweep now notices folders your own `.git/info/exclude` lists, not just the ones in the
  project's `.gitignore`. Previously those were only picked up on Windows.
- **A scan now shows what it's doing.** A bar along the bottom fills as it goes, names the step it's
  on ("Looking through your project folders"), counts what it has found so far, and lists the folders
  it's looking at as it reaches them. A long scan no longer looks like a frozen window.
- **Cleaning up shows its progress, and items leave the list as they go.** Each row disappears the
  moment what it covers has actually been removed, with a running count and total along the bottom.
  Anything that couldn't be removed stays put and says why, instead of vanishing silently.
- **Cleaning up no longer re-scans afterwards.** It used to run a second full scan while the old list
  was still on screen, so everything you had just removed sat there as though nothing had happened.
  The list is now corrected directly, which is instant, and you can clean up again straight away.
- **The bar along the bottom stays put.** It shows what you've selected and the Clean up button
  wherever you are in a long list, rather than only appearing once something is ticked.
- The dock icon (or the taskbar button on Windows) fills up while a scan or a clean-up runs, so you
  can leave ChillSweep working in the background and still see how far along it is.
- The scan no longer says Steam wasn't found. Not having a program installed is the normal case, and
  saying so pushed the notes that do need acting on further down.
- **Scanning is a lot faster.** Working out whether a folder is empty no longer measures everything
  inside it first — it stops at the first file it finds, and checks folders side by side rather than
  one at a time. On a machine with a few large folders in the usual places that alone cut a scan
  from about 75 seconds to about 45.
## 1.21.0 — 2026-10-03

- **ChillSweep now runs on macOS.** Signed and notarized, so it opens without warnings. It works out
  what's installed from your app bundles and installer receipts, and knows where macOS keeps app data,
  caches, logs and reopened-window state.
- Removal goes to the Trash. macOS gives no way to put things back from inside an app, so instead of
  an Undo button you get **Show in Trash**, and Finder puts them back.
- The app says Trash, Finder and apps on macOS, rather than Recycle Bin, Explorer and Program Files.
- macOS asks before ChillSweep reads your Desktop, Documents or Downloads. If you say no, the scan
  tells you which folder it couldn't check instead of quietly reporting nothing. ChillSweep never asks
  for Full Disk Access.
- **New Projects section.** ChillSweep now recognises project folders from creative tools and game
  engines — Pro Tools, REAPER, Ableton, Cubase, Studio One, Bitwig, Logic, Premiere, DaVinci Resolve,
  Unity, Unreal, Godot, Blender and LaTeX — by the project file inside them, so they're no longer
  mistaken for leftovers. What it finds about them sits in its own section below the usual three and
  stays out of the main total: the parts each tool rebuilds by itself (waveform and peak caches,
  preview and proxy media, session backups, engine import caches), projects that were created and
  never used, and projects nothing has opened in over a year. Each still says whether it's safe to
  clear or your call, and nothing still holding recorded or imported media is ever called unused.
- Added a lot more known caches on both platforms: the Adobe media cache that Premiere and After
  Effects fill up, After Effects' disk cache, Xcode build data and simulator devices, Android emulator
  images, Unreal and Unity shared caches, Microsoft Teams, Homebrew, and the caches kept by pnpm, bun,
  Cypress, Puppeteer, Poetry, uv, conda, Go, Maven and Flutter.
- Fixed items found in Documents being listed but never actually removable. ChillSweep reported them
  and then silently skipped them.
- Fixed empty folders going unreported whenever one of them was somewhere ChillSweep refuses to touch.
- Fixed Electron's caches being counted twice in the "safe to clear" total.

## 1.10.0 — 2026-09-30

- Removed `chillsweep-scan.exe` from the install folder. It's a command-line version of the scan that shipped before it was ready, and updating removes it.

## 1.5.0 — 2026-09-30

First public release.

- ChillSweep checks your user folders against what's actually installed, so it can find what uninstalled apps, old app versions, caches, dev projects and games left behind.
- Every finding comes with plain-language evidence ("Docker Desktop isn't installed") and what happens if you remove it, sorted into **Safe to clear**, **Leftovers** and **Your call**.
- Nothing is selected for you, removals go to the Recycle Bin with an **Undo** button, and each item is re-checked just before it's removed.
- ChillSweep updates itself: when a new version is out it offers to install it, and About → **What's new** lists what changed.
