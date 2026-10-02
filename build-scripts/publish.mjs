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
import { client, expectOk, FEED, listReleases, mergeFeed, mine, OWNER, REPO, STAGE, token } from "./github.mjs";
import { git, root, version } from "./version.mjs";

async function main() {
  const tok = token();
  if (!tok) throw new Error("no GH_TOKEN (release.env or the environment) — see RELEASING.md.");
  const api = client(tok);

  const ver = version();
  const tag = `v${ver}`;

  // ── 1. Stage ────────────────────────────────────────────────────────────────
  // Everything below is per-platform. On Windows the thing people download is also the thing the
  // updater fetches; on macOS they differ — the DMG is for humans, the .app.tar.gz for the updater —
  // so both are staged and uploaded.
  const art = mine();
  const target = process.platform === "darwin" ? join("target", "universal-apple-darwin") : "target";
  const bundle = join(root, "src-tauri", target, "release", ...art.bundleDir);
  const built = join(bundle, art.built(ver));
  if (!existsSync(built)) throw new Error(`no ${built} — run the release build first (npm run release).`);

  // The updater artifact and its signature. Tauri writes these next to the bundle it made them from.
  const updaterSrc = process.platform === "darwin" ? join(root, "src-tauri", target, "release", "bundle", "macos", `ChillSweep.app.tar.gz`) : built;
  if (!existsSync(`${updaterSrc}.sig`)) {
    throw new Error(`no ${updaterSrc}.sig — the build wasn't signed for the updater (TAURI_SIGNING_PRIVATE_KEY).`);
  }

  // Cleared first: a stale file in dist/ would otherwise be uploaded, or hashed into the notes.
  rmSync(STAGE, { recursive: true, force: true });
  mkdirSync(STAGE);
  copyFileSync(built, join(STAGE, art.download));
  const staged = [art.download];
  if (art.updater !== art.download) {
    copyFileSync(updaterSrc, join(STAGE, art.updater));
    staged.push(art.updater);
  }
  copyFileSync(`${updaterSrc}.sig`, join(STAGE, `${art.updater}.sig`));
  staged.push(`${art.updater}.sig`);

  const signature = readFileSync(`${updaterSrc}.sig`, "utf8").trim();
  // This release's own asset, not /latest/: the feed must keep pointing at the file it was written
  // for, whichever release is newest when someone reads it.
  const updaterUrl = `https://github.com/${OWNER}/${REPO}/releases/download/${tag}/${art.updater}`;
  const myPlatforms = Object.fromEntries(art.feedKeys.map((k) => [k, { signature, url: updaterUrl }]));

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

  // ── 3. Merge this platform into the feed ────────────────────────────────────
  // The feed is shared: Windows and macOS each publish into the same latest.json, from different
  // machines. Writing it from scratch — which this did — means whichever runs second erases the
  // other's platform entry, and nothing anywhere reports an error. The symptom is simply that one
  // platform stops being offered updates.
  //
  // So read what is already on the draft and merge into it.
  const existingAsset = release.assets.find((a) => a.name === FEED);
  let prior = null;
  if (existingAsset) {
    const res = await api(`/releases/assets/${existingAsset.id}`, { headers: { Accept: "application/octet-stream" } });
    // A transient failure here must NOT read as "there was no prior feed": that would merge into
    // nothing and upload a feed holding only this machine's platform, which is the exact silent
    // one-platform-stops-updating failure this merge exists to prevent.
    if (!res.ok) {
      throw new Error(
        `couldn't read the ${FEED} already on ${tag} (HTTP ${res.status} ${res.statusText}). ` +
          `Refusing to continue: writing a fresh feed would drop the other platform's entry. Retry, or delete that asset to start it over.`,
      );
    }
    const text = await res.text();
    try {
      prior = text.trim() ? JSON.parse(text) : null;
    } catch {
      throw new Error(`the ${FEED} already on ${tag} is not valid JSON. Delete that asset and release again.`);
    }
    if (prior) console.log(`publish: merging into the existing feed (had ${Object.keys(prior.platforms ?? {}).join(", ") || "no platforms"})`);
  }
  const feed = mergeFeed(prior, myPlatforms, {
    version: ver,
    notes: changelogSection("Unreleased") || "",
    pubDate: new Date().toISOString(),
  });
  writeFileSync(join(STAGE, FEED), JSON.stringify(feed, null, 2) + "\n");
  console.log(`publish: feed now covers ${Object.keys(feed.platforms).join(", ")}`);

  // ── 4. Upload ───────────────────────────────────────────────────────────────
  for (const name of [...staged, FEED]) {
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
