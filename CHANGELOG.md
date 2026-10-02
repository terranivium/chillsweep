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

- **ChillSweep now runs on macOS.** Signed and notarized, so it opens without warnings. It reads your
  app bundles and installer receipts to work out what's actually installed, and knows where macOS keeps
  app data, caches, logs and reopened-window state.
- Finds a lot more on macOS: Xcode build data and simulator devices, the Adobe media cache that Premiere
  and After Effects fill up, Homebrew and language-tool caches, and per-app caches in your Library.
- Removal goes to the Trash. macOS gives no way to put things back from inside an app, so instead of an
  Undo button you get **Show in Trash**, and Finder puts them back.
- The app now says Trash, Finder and apps on macOS, rather than Recycle Bin, Explorer and Program Files.
- macOS asks before ChillSweep reads your Desktop, Documents or Downloads. If you say no, the scan says
  which folder it couldn't check instead of quietly reporting nothing. ChillSweep never asks for Full
  Disk Access.

- ChillSweep now recognises project folders from creative tools and game engines — Pro Tools, REAPER, Ableton, Cubase, Studio One, Bitwig, Logic, Premiere, DaVinci Resolve, Unity, Unreal, Godot, Blender and LaTeX — by the project file inside them. They're no longer mistaken for leftovers, and the parts each tool rebuilds by itself (waveform and peak caches, preview and proxy media, session backups, engine import caches) are offered as safe to clear.
- Projects that were created and never used, and projects nothing has opened in over a year, are now grouped into findings you can look through. Nothing that still holds recorded or imported media is ever called unused.
- Added a lot more known caches, including the Adobe media cache that Premiere and After Effects fill up, After Effects' disk cache, Xcode simulator devices, Android emulator images, Unreal and Unity shared caches, Microsoft Teams, and the caches kept by pnpm, bun, Cypress, Puppeteer, Poetry, uv, conda, Go, Maven and Flutter.
- Fixed items found in Documents being listed but never actually removable. ChillSweep reported them and then silently skipped them.
- Unity and Unreal build folders inside a git project are now correctly described as caches the engine rebuilds, instead of being called local files that can't be recreated.

## 1.10.0 — 2026-09-30

- Removed `chillsweep-scan.exe` from the install folder. It's a command-line version of the scan that shipped before it was ready, and updating removes it.

## 1.5.0 — 2026-09-30

First public release.

- ChillSweep checks your user folders against what's actually installed, so it can find what uninstalled apps, old app versions, caches, dev projects and games left behind.
- Every finding comes with plain-language evidence ("Docker Desktop isn't installed") and what happens if you remove it, sorted into **Safe to clear**, **Leftovers** and **Your call**.
- Nothing is selected for you, removals go to the Recycle Bin with an **Undo** button, and each item is re-checked just before it's removed.
- ChillSweep updates itself: when a new version is out it offers to install it, and About → **What's new** lists what changed.
