// The one version every release surface agrees on: {major}.{git rev-list --count HEAD}.0.
//
//   node build-scripts/version.mjs     → prints it
//
// Same scheme as Vocal Slice. The major comes from package.json, which otherwise stays a static
// "1.0.0"; the commit count makes every commit a higher version, so the updater always sees a newer
// release as newer. pack.mjs bakes it into the build, publish.mjs tags the release with it, and
// latest.json announces it, all through this file, so the three can't disagree.
//
// Consequence worth knowing: two releases from the same commit get the same version, and the second
// is never offered as an update. Make a commit between releases.

import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export const root = join(dirname(fileURLToPath(import.meta.url)), "..");

export function git(args) {
  const r = spawnSync("git", args, { cwd: root, encoding: "utf8" });
  return r.status === 0 ? r.stdout.trim() : null;
}

/** The commit count, or null outside a git checkout. */
export function buildNumber() {
  const n = git(["rev-list", "--count", "HEAD"]);
  return n && /^\d+$/.test(n) ? n : null;
}

export function version() {
  const pkg = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
  const major = String(pkg.version).split(".")[0] || "1";
  const n = buildNumber();
  return n ? `${major}.${n}.0` : String(pkg.version);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  console.log(version());
}
