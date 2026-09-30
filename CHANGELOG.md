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

## 1.10.0 — 2026-09-30

- Removed `chillsweep-scan.exe` from the install folder. It's a command-line version of the scan that shipped before it was ready, and updating removes it.

## 1.5.0 — 2026-09-30

First public release.

- ChillSweep checks your user folders against what's actually installed, so it can find what uninstalled apps, old app versions, caches, dev projects and games left behind.
- Every finding comes with plain-language evidence ("Docker Desktop isn't installed") and what happens if you remove it, sorted into **Safe to clear**, **Leftovers** and **Your call**.
- Nothing is selected for you, removals go to the Recycle Bin with an **Undo** button, and each item is re-checked just before it's removed.
- ChillSweep updates itself: when a new version is out it offers to install it, and About → **What's new** lists what changed.
