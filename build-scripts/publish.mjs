// Uploads the release build into a DRAFT GitHub release on this repo.
//
//   node build-scripts/publish.mjs     (chained into npm run release, after pack.mjs --release)
//
// Vocal Slice gets this from electron-builder's `--publish always`; the Tauri CLI only builds, so this
// is the equivalent. It:
//
//   1. stages dist/ with exactly what ships: ChillSweep-Setup.exe, its .sig, and latest.json
//   2. finds the draft tagged v{version}, or creates it, targeting the exact commit that was built
//      (GitHub creates the tag there when you publish)
//   3. uploads the three files, replacing same-named assets so a re-run is safe
//
// It never publishes. Publishing is what ships the update to every installed copy (the app reads
// /releases/latest/download/latest.json, which ignores drafts), so it stays a deliberate manual step.

import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { changelogSection } from "./changelog-data.mjs";
import { client, expectOk, FEED, INSTALLER, listReleases, OWNER, REPO, SIGNATURE, STAGE, token } from "./github.mjs";
import { git, root, version } from "./version.mjs";

async function main() {
  const tok = token();
  if (!tok) throw new Error("no GH_TOKEN (release.env or the environment) — see RELEASING.md.");
  const api = client(tok);

  const ver = version();
  const tag = `v${ver}`;

  // ── 1. Stage ────────────────────────────────────────────────────────────────
  const bundle = join(root, "src-tauri", "target", "release", "bundle", "nsis");
  const exe = join(bundle, `ChillSweep_${ver}_x64-setup.exe`);
  if (!existsSync(exe)) throw new Error(`no ${exe} — run the release build first (npm run release).`);
  if (!existsSync(`${exe}.sig`)) throw new Error(`no ${exe}.sig — the build wasn't signed for the updater.`);

  // Cleared first: a stale file in dist/ would otherwise be uploaded, or hashed into the notes.
  rmSync(STAGE, { recursive: true, force: true });
  mkdirSync(STAGE);
  copyFileSync(exe, join(STAGE, INSTALLER));
  copyFileSync(`${exe}.sig`, join(STAGE, SIGNATURE));

  const feed = {
    version: ver,
    notes: changelogSection("Unreleased") || "",
    pub_date: new Date().toISOString(),
    platforms: {
      "windows-x86_64": {
        signature: readFileSync(`${exe}.sig`, "utf8").trim(),
        // This release's own asset, not /latest/: the feed must keep pointing at the installer it was
        // written for, whichever release is newest when someone reads it.
        url: `https://github.com/${OWNER}/${REPO}/releases/download/${tag}/${INSTALLER}`,
      },
    },
  };
  writeFileSync(join(STAGE, FEED), JSON.stringify(feed, null, 2) + "\n");

  // ── 2. Find or create the draft ─────────────────────────────────────────────
  let release = (await listReleases(api)).find((r) => r.tag_name === tag);
  if (release && !release.draft) {
    throw new Error(`${tag} is already published. Make a commit so the version moves on, then release again.`);
  }
  if (!release) {
    release = await (
      await expectOk(
        await api("/releases", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            tag_name: tag,
            // Without this the tag lands on whatever main is when you press Publish, not this build.
            target_commitish: git(["rev-parse", "HEAD"]),
            name: `ChillSweep ${ver}`,
            draft: true,
            body: "",
          }),
        }),
      )
    ).json();
    console.log(`publish: created draft ${tag}`);
  } else {
    console.log(`publish: found draft ${tag}`);
  }

  // ── 3. Upload ───────────────────────────────────────────────────────────────
  for (const name of [INSTALLER, SIGNATURE, FEED]) {
    const old = release.assets.find((a) => a.name === name);
    if (old) await expectOk(await api(`/releases/assets/${old.id}`, { method: "DELETE" }));
    const data = readFileSync(join(STAGE, name));
    await expectOk(
      await api(`https://uploads.github.com/repos/${OWNER}/${REPO}/releases/${release.id}/assets?name=${encodeURIComponent(name)}`, {
        method: "POST",
        headers: { "Content-Type": name.endsWith(".json") ? "application/json" : "application/octet-stream" },
        body: data,
      }),
    );
    console.log(`publish: uploaded ${name.padEnd(26)} ${(data.length / 1048576).toFixed(1).padStart(6)} MB${old ? "  (replaced)" : ""}`);
  }
  console.log(`publish: ${tag} is a draft — nothing is live until you publish it by hand.`);
}

main().catch((err) => {
  console.error(`publish failed: ${err.message}`);
  process.exitCode = 1;
});
