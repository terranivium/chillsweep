// Builds the Windows installer with the real version baked in.
//
//   node build-scripts/pack.mjs             (npm run build)    local build, no signing key needed
//   node build-scripts/pack.mjs --release   (npm run release)  also signs the updater artifacts
//
// tauri.conf.json keeps a static "1.0.0" (what `tauri dev` reports). Here it's overridden with
// 1.{commit count}.0 via `tauri build --config`, which is what the About screen shows and what the
// updater compares. Done in a wrapper rather than an inline env var so it works the same from
// PowerShell, cmd and bash.
//
// --release adds bundle.createUpdaterArtifacts, which makes Tauri write ChillSweep_…-setup.exe.sig
// next to the installer. That needs TAURI_SIGNING_PRIVATE_KEY (and its password) from release.env,
// so it's only switched on for releases.

import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { join } from "node:path";
import { loadReleaseEnv } from "./github.mjs";
import { git, root, version } from "./version.mjs";

const release = process.argv.includes("--release");
const config = { version: version() };

if (release) {
  loadReleaseEnv();
  const conf = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"));
  if (!conf.plugins?.updater?.pubkey) {
    console.error("pack: plugins.updater.pubkey in src-tauri/tauri.conf.json is empty — see RELEASING.md, one-time setup.");
    process.exit(1);
  }
  if (!process.env.TAURI_SIGNING_PRIVATE_KEY) {
    console.error("pack: TAURI_SIGNING_PRIVATE_KEY is not set (release.env or the environment) — see RELEASING.md.");
    process.exit(1);
  }
  // The release is tagged at this commit in the public repo, so the installer must be built from exactly
  // what's there: nothing uncommitted, and the commit already pushed (GitHub can't tag a commit it hasn't
  // got). Checked here rather than in publish.mjs so it fails before the build, not after.
  if (git(["status", "--porcelain"])) {
    console.error("pack: the working tree has uncommitted changes. Commit (and push) before releasing.");
    process.exit(1);
  }
  git(["fetch", "--quiet", "origin"]);
  if (!git(["branch", "--remotes", "--contains", "HEAD"])) {
    console.error("pack: HEAD isn't on GitHub yet. Push it before releasing.");
    process.exit(1);
  }
  config.bundle = { createUpdaterArtifacts: true };

  // macOS releases are signed and notarized, and there is no unsigned fallback: Gatekeeper
  // simply refuses an unnotarized download. Missing credentials would otherwise surface as a
  // confusing bundler failure half an hour into a build.
  //
  // The signing certificate is NOT here — it comes from the login keychain, named by
  // bundle.macOS.signingIdentity. These three are only for submitting to Apple's notary service.
  if (process.platform === "darwin") {
    const missing = ["APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID"].filter((k) => !process.env[k]);
    if (missing.length) {
      console.error(`pack: ${missing.join(", ")} not set (release.env or the environment) — see RELEASING.md.`);
      process.exit(1);
    }
  }
}

console.log(`pack: building ChillSweep ${config.version}${release ? " (release, signed for the updater)" : ""}`);
const cli = join(root, "node_modules", "@tauri-apps", "cli", "tauri.js");
// One universal binary covers Intel and Apple Silicon, so there is a single DMG to download and
// a single updater artifact to sign. Note this moves the output under
// target/universal-apple-darwin/, which publish.mjs accounts for.
const args = ["build", "--config", JSON.stringify(config)];
if (process.platform === "darwin") {
  args.push("--target", "universal-apple-darwin");
}
const result = spawnSync(process.execPath, [cli, ...args], {
  stdio: "inherit",
  cwd: root,
});
process.exit(result.status ?? 1);
