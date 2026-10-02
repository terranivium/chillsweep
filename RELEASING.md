# Releasing ChillSweep

A release is built on **two machines** — Windows for the installer, macOS for the signed universal
DMG — and lands as a single GitHub Release on this repo,
[terranivium/chillsweep](https://github.com/terranivium/chillsweep), tagged at the commit it was built
from. The process is Vocal Slice's, minus the separate releases repo: ChillSweep is open source
(GPL-3.0), so the source and its releases live together.

Publishing **is** the update mechanism: there's no separate step and nothing is pushed to users. Each
release carries `latest.json`, installed copies read
`/releases/latest/download/latest.json` on startup, and they offer the update when its version is
higher. Drafts are invisible there, so nothing reaches anyone until you press Publish.

## Versions

The version is `1.{git rev-list --count HEAD}.0` (`build-scripts/version.mjs`). `package.json` and
`tauri.conf.json` stay at a static `1.0.0`, which is what `tauri dev` reports. `npm run build` and
`npm run release` bake the real number in.

**The version must actually increase.** Two releases from the same commit get the same version, and
installed copies will never be offered the second. Make a commit between releases. `publish.mjs`
refuses to touch a tag that's already published.

## One-time setup — **on each machine**

`release.env` is gitignored and does not travel with the repo, so both machines need their own copy.
They need different contents: `GH_TOKEN` and the updater key on both, and the `APPLE_*` notarization
variables on the Mac only. The Mac additionally needs the **Developer ID Application** certificate in
its login keychain — that is named by `bundle.macOS.signingIdentity` and is deliberately not an
environment variable. The same GitHub token works on both; it is tied to the account, not the machine.


1. **Make sure the repo is public.** Installed copies fetch `latest.json` and the installer anonymously,
   so updates can't work from a private repo.
2. **Create a token.** Use a fine-grained personal access token scoped to only `terranivium/chillsweep`,
   with **Contents: Read and write** (enough to create releases and upload assets). Fine-grained tokens
   **expire**, so if a release suddenly fails with a 401, check expiry first.
3. **Generate the updater signing key:**

   ```sh
   npm run tauri signer generate -- -w "$HOME/.tauri/chillsweep.key"
   ```

   Paste the printed **public** key into `src-tauri/tauri.conf.json` → `plugins.updater.pubkey` and
   commit it. The private key stays in `C:\Users\<you>\.tauri\`, outside the repo. Use `$HOME`, which
   works in PowerShell and Git Bash. `%USERPROFILE%` only expands in cmd; anywhere else it creates a
   folder literally named that, inside the repo (`.gitignore` blocks `*.key` as a backstop).

   > ⚠ **Back the private key and its password up somewhere safe.** Every installed copy only accepts
   > updates signed with it. Lose it and those copies can never update again: users would have to
   > download and reinstall by hand.
4. **Fill in `release.env`:**

   ```sh
   copy release.env.example release.env
   ```

   It's gitignored and read by the build scripts. It holds `GH_TOKEN`, `TAURI_SIGNING_PRIVATE_KEY` (the
   key file's path) and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Real environment variables win over the
   file. Never commit it.

**Node 22 or newer**, because the scripts use global `fetch` and have no dependencies.

## Steps

1. **Commit and push**, including the `CHANGELOG.md` bullets. The release refuses to build from a
   working tree with uncommitted changes, or from a commit GitHub doesn't have yet. It gets tagged at
   that commit, so the public source must match the installer. If dependencies changed, run
   `npm run notices` first and commit the regenerated `src/third-party-notices.txt`.

2. **⚠ Both machines must be on the same commit.** The version is
   `1.{git rev-list --count HEAD}.0`, so a Windows box and a Mac sitting on different commits
   produce **different version numbers** and therefore **two separate releases**, each missing the
   other's platform. Before building, on both:

   ```
   git rev-list --count HEAD      # must be IDENTICAL on both machines
   ```

   `git pull` is not enough if one is on a branch — check out the same commit explicitly.
   `publish.mjs` refuses to merge a feed whose version differs from the build, so the failure is
   loud rather than silent, but it is cheaper to catch it here.

   **Order doesn't matter.** Whichever machine runs first creates the draft and writes "What's new";
   the second finds the draft and adds its own assets and platform keys to the existing
   `latest.json`. A log line saying `created draft` on the *second* machine means the two were on
   different commits and you now have two drafts.

3. **`npm run release`** on each machine. This runs three scripts in order:

   | Script | Does |
   | --- | --- |
   | `pack.mjs --release` | checks the tree is clean and pushed, then builds at the real version and signs for the updater (`.sig`). On Windows the NSIS installer; on macOS a universal `.app` and DMG, signed with the Developer ID from the login keychain and notarized |
   | `publish.mjs` | stages `dist/` with this platform's assets, finds or creates the **draft** `v1.N.0` targeting the built commit, **merges** this platform's keys into any `latest.json` already there, and uploads |
   | `release-notes.mjs` | writes "What's new" from `CHANGELOG.md` and the SHA-256 table into the draft, merging checksum rows by filename so each machine adds its own |

   Nothing is published and no tag exists yet; GitHub creates the tag when you publish. Re-running is
   safe.

   The installer is uploaded under the stable name `ChillSweep-Setup.exe`, so
   `https://github.com/terranivium/chillsweep/releases/latest/download/ChillSweep-Setup.exe` always
   fetches the newest one.

4. **"What's new" comes from `CHANGELOG.md`**, which is authoritative for every release, past and
   present. Add bullets under `## Unreleased` *as you make the change*. **Never write notes in the
   GitHub UI**: edit the file and push, so the two can't diverge.

   The two blocks in the release body follow different rules:

   | Block | Rule |
   | --- | --- |
   | `<!-- whatsnew:start -->` … | **Written only when absent.** Once it exists, generated or typed, nothing overwrites it. `--refresh` forces a re-pull |
   | `<!-- checksums:start -->` … | **Always regenerated** from `dist/` |

   If `## Unreleased` is empty, the script falls back to commit subjects since the previous release and
   warns. Treat that as a prompt to write the changelog, then run `npm run release-notes -- --refresh`.

   | Task | Command |
   | --- | --- |
   | Fix the release in flight | `npm run release-notes -- --refresh` |
   | Fix an already-published release | `npm run release-notes -- --notes=1.42.0` |
   | Close a version out, after publishing | `npm run changelog:promote` |

   `--notes` rewrites only that release's "What's new". It never touches the checksum table, because the
   table is hashed from whatever is in `dist/` *now*, and those would be the wrong hashes for an old
   release.

5. **`npm run release:check`.** This is read-only and exits non-zero if publishing would be a mistake.
   It asserts:
   - `ChillSweep-Setup.exe`, `ChillSweep-Setup.exe.sig` and `latest.json` are all on the release
   - `latest.json`'s version matches the tag
   - its URL points at *this* tag's installer
   - its signature matches the `.sig`
   - "What's new" isn't the `- …` placeholder
   - the checksum table covers the installer

   A broken feed fails **silently** in the field: downloads keep working and updates never arrive. That's
   why this check exists, so ask it, not the build log. `-- --tag=v1.42.0` inspects an older release.

6. **Publish the draft** on GitHub. Updates start flowing to installed copies from this moment.

7. **Close the version out: `npm run changelog:promote`.** It renames `## Unreleased` to
   `## 1.N.0 — YYYY-MM-DD` and opens a fresh empty one above it. It only edits the local file and
   doesn't commit, so review the diff, then commit and push it. Skip this and the next release republishes these
   notes; the script warns when it spots that.

## After the Mac build, before publishing

Gatekeeper refuses an unnotarized download outright, and the updater rejects an unnotarized
replacement — there is no unsigned fallback on macOS, and both fail without a useful message. So
check the artifacts on the Mac:

```
npm run release:check                                  # asserts stapling, among everything else
xcrun stapler validate dist/ChillSweep.dmg
lipo -info src-tauri/target/universal-apple-darwin/release/bundle/macos/ChillSweep.app/Contents/MacOS/ChillSweep
codesign -dvvv --entitlements - "src-tauri/target/universal-apple-darwin/release/bundle/macos/ChillSweep.app" 2>&1 | grep -E 'Authority|Runtime'
```

`lipo` must print `x86_64 arm64`. `syspolicy_check distribution dist/ChillSweep.dmg` (macOS 14+) is
Apple's own pre-distribution linter and names a missing signature or ticket outright, which
`release:check` runs for you where available.

**Tauri's DMG stapling is unverified.** It staples the `.app`; whether the ticket also lands on the
disk image is undocumented, and electron-builder got this wrong in Vocal Slice while every other
check passed. `release:check` refuses to vouch for an unstapled DMG, so trust the check rather than
the assumption — and if the DMG comes back unticketed, add an `xcrun notarytool submit --wait` plus
`xcrun stapler staple` step to `pack.mjs`.

## Afterwards

- Confirm `…/releases/latest/download/ChillSweep-Setup.exe` downloads.
- **Test the updater.** Install the previous version, publish this one, and launch the old copy. You
  should see "ChillSweep 1.N.0 is available", then **Update and restart** installs it and reopens.
  The new version then says "Updated to ChillSweep 1.N.0" once, with a link to What's new.

## Notes

- The **Windows** installer isn't code-signed, so SmartScreen warns on first run. The release notes say
  so and give the SHA-256 to verify with. The **macOS** build is signed and notarized, so it opens
  normally. The updater's own signature check is separate from both and always enforced.
- The updater's minisign key must be the **same on both machines** — one `pubkey` in `tauri.conf.json`
  serves every platform, so a build signed with a different key produces updates the other platform's
  copies reject.
- `latest.json` is shared. `publish.mjs` reads the draft's existing copy and merges, so neither machine
  clobbers the other. If it ever did, the symptom would be silent: that platform simply stops being
  offered updates. `npm test` covers that merge — worth running after touching `build-scripts/`.
- `npm run build` builds without the key and without publishing. Use it for local testing. It writes to
  `src-tauri/target/release/bundle/nsis/`.
- The in-app What's new needs nothing here: every build and `tauri dev` bake `src/changelog.json` from
  `CHANGELOG.md` (tauri.conf.json's `beforeBuildCommand` / `beforeDevCommand`).
- On Windows only the NSIS installer is built. MSI is off, because MSI caps the middle version number at
  255, which the commit count will pass. Targets are set per platform in
  `src-tauri/tauri.windows.conf.json` and `src-tauri/tauri.macos.conf.json`, which Tauri merges over
  `tauri.conf.json` — left unset, Tauri would bundle everything it can, MSI included.
