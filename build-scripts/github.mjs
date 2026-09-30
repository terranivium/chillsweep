// Shared plumbing for the scripts that talk to GitHub: secrets, the API, and the names of the files a
// release carries. Releases live on the source repo itself. Node 22+ global fetch, no dependencies.

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { root } from "./version.mjs";

export const OWNER = "terranivium";
export const REPO = "chillsweep";

// Stable names, so https://github.com/…/releases/latest/download/ChillSweep-Setup.exe always works.
// Tauri names its output ChillSweep_1.N.0_x64-setup.exe; publish.mjs renames on upload.
export const INSTALLER = "ChillSweep-Setup.exe";
export const SIGNATURE = `${INSTALLER}.sig`;
export const FEED = "latest.json";

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
