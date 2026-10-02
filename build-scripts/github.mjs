// Shared plumbing for the scripts that talk to GitHub: secrets, the API, and the names of the files a
// release carries. Releases live on the source repo itself. Node 22+ global fetch, no dependencies.

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { root } from "./version.mjs";

export const OWNER = "terranivium";
export const REPO = "chillsweep";

// What each platform contributes to a release. Stable names, so
// https://github.com/…/releases/latest/download/ChillSweep-Setup.exe always works; Tauri's own output
// is named per version (ChillSweep_1.N.0_x64-setup.exe), and publish.mjs renames on upload.
//
// `download` is what a person clicks. `updater` is what installed copies fetch, which on macOS is a
// different file from the download: the DMG is for humans, the .app.tar.gz is for the updater.
// `feedKeys` are the platform keys written into latest.json.
export const ARTIFACTS = {
  win32: {
    download: "ChillSweep-Setup.exe",
    updater: "ChillSweep-Setup.exe",
    bundleDir: ["bundle", "nsis"],
    // Tauri's own filename inside bundleDir, given the version.
    built: (v) => `ChillSweep_${v}_x64-setup.exe`,
    feedKeys: ["windows-x86_64"],
    label: "Windows",
  },
  darwin: {
    download: "ChillSweep.dmg",
    updater: "ChillSweep-macOS-update.app.tar.gz",
    bundleDir: ["bundle", "dmg"],
    built: (v) => `ChillSweep_${v}_universal.dmg`,
    // One universal build serves both architectures, so every key points at the same file.
    // Tauri documents only the two arch-specific keys; darwin-universal is there for clients
    // that look it up.
    feedKeys: ["darwin-universal", "darwin-aarch64", "darwin-x86_64"],
    label: "macOS",
  },
};

/** Which platform this machine builds for. */
export const plat = () => (process.platform === "darwin" ? "darwin" : "win32");

/** This machine's artifact set. */
export const mine = () => ARTIFACTS[plat()];

export const FEED = "latest.json";

/**
 * Merge one platform's entries into a shared update feed.
 *
 * Both machines publish into the same `latest.json`. Writing it from scratch — which publish.mjs
 * used to — means whichever runs second erases the other's platform entry, and nothing reports an
 * error: that platform simply stops being offered updates. Hence a merge.
 *
 * `prior` is whatever is already on the draft, or null. Throws if `prior` is for a different
 * version, because that means the two machines are on different commits and their artifacts belong
 * to different builds — merging would publish a feed pointing one platform at a version it never
 * built.
 *
 * Extracted and exported so it can be tested without a release; see build-scripts/feed.test.mjs.
 */
export function mergeFeed(prior, platforms, { version, notes, pubDate }) {
  if (prior && prior.version !== version) {
    throw new Error(
      `the ${FEED} already on this release is for version ${prior.version}, this build is ${version}. ` +
        `The two machines are on different commits — check out the same one on both (see RELEASING.md) and retry.`,
    );
  }
  const base = prior ?? { version, notes, pub_date: pubDate, platforms: {} };
  return { ...base, platforms: { ...(base.platforms ?? {}), ...platforms } };
}

// Where publish.mjs stages exactly what gets uploaded, and release-notes.mjs hashes it from.
export const STAGE = join(root, "dist");

/**
 * Load release.env (gitignored) into process.env. Real environment variables win, so the file is
 * optional for anyone who'd rather not keep secrets on disk.
 */
export function loadReleaseEnv() {
  const file = join(root, "release.env");
  if (!existsSync(file)) return;
  for (const line of readFileSync(file, "utf8").split(/\r?\n/)) {
    const m = line.match(/^\s*([A-Z0-9_]+)\s*=\s*(.*?)\s*$/);
    if (m && m[2] !== "" && process.env[m[1]] === undefined) process.env[m[1]] = m[2];
  }
}

export function token() {
  loadReleaseEnv();
  return process.env.GH_TOKEN || null;
}

/** fetch() against the repo, authenticated. `p` is the path after /repos/{owner}/{repo}. */
export function client(tok) {
  return (p, init = {}) =>
    fetch(p.startsWith("https://") ? p : `https://api.github.com/repos/${OWNER}/${REPO}${p}`, {
      ...init,
      headers: { Authorization: `Bearer ${tok}`, Accept: "application/vnd.github+json", ...init.headers },
    });
}

export async function expectOk(res) {
  if (res.ok) return res;
  const hint = res.status === 401 ? " — token rejected. Fine-grained tokens expire; check that first." : "";
  throw new Error(`GitHub API ${res.status} ${res.statusText}${hint}: ${await res.text()}`);
}

/** Every release, drafts included. Drafts only appear on the authenticated list endpoint. */
export async function listReleases(api) {
  return (await expectOk(await api("/releases?per_page=100"))).json();
}
