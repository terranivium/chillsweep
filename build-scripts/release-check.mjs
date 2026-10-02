// Pre-publish verification for a ChillSweep release. Read-only: never patches, uploads or publishes.
//
//   npm run release:check
//   npm run release:check -- --tag=v1.42.0     (an older release)
//
// Why a script rather than a checklist: a release missing latest.json, or with a feed that points at
// the wrong installer or carries the wrong signature, breaks auto-update SILENTLY. Downloads keep
// working and updates simply never arrive. So ask the API what is really on the release.
//
// Exit code is 1 if anything should block publishing.

import { spawnSync } from "node:child_process";
import { existsSync, statSync } from "node:fs";
import { join } from "node:path";
import { ARTIFACTS, client, FEED, listReleases, mine, OWNER, REPO, STAGE, token } from "./github.mjs";
import { DOWNLOADABLE, END, parseRows, SEED, START, whatsNewProse } from "./release-notes.mjs";
import { root, version } from "./version.mjs";

// A finished release carries both platforms' assets. Either machine can run this check, so a
// missing asset might just mean the other one hasn't published yet — the message says so.
const EXPECTED = [
  ...Object.values(ARTIFACTS).flatMap((a) => {
    const items = [{ name: a.download, note: `the ${a.label} download` }];
    if (a.updater !== a.download) items.push({ name: a.updater, note: `what ${a.label} copies update from` });
    items.push({ name: `${a.updater}.sig`, note: `the updater refuses a ${a.label} build without it` });
    return items;
  }),
  { name: FEED, note: "the update feed installed copies read" },
];

const problems = [];
const fail = (m) => problems.push(m);
/// Things this run could NOT check, as opposed to things that failed. Printed with the verdict,
/// because "Ready to publish" is misleading if notarization was never actually tested.
const notes = [];

/**
 * Is the macOS build actually notarized and stapled?
 *
 * This matters more than it looks. Gatekeeper refuses an unstapled download outright, and the
 * updater replaces the .app with the one inside the tarball — so all three artifacts need a
 * ticket, not just the one Tauri happened to staple. Vocal Slice learned this the hard way:
 * electron-builder notarized the app and left the DMG unticketed while every other check passed.
 *
 * Necessarily a local check — verifying the uploaded copy would mean downloading it — so it
 * first confirms the local file IS the uploaded one before vouching for anything.
 */
function checkMacSigning(byName) {
  if (process.platform !== "darwin") {
    console.log("  ..  not checked            needs macOS — run release:check on the Mac before publishing");
    notes.push("macOS signing unverified on this machine (not macOS)");
    return;
  }
  const art = mine();
  const dmg = join(STAGE, art.download);
  if (!existsSync(dmg)) {
    console.log(`  ..  not checked            no dist/${art.download} on this machine`);
    notes.push("macOS signing unverified — nothing in dist/ to inspect");
    return;
  }
  // A local artifact only speaks for the release if it IS the released artifact.
  const asset = byName.get(art.download);
  const localSize = statSync(dmg).size;
  if (asset && asset.size !== localSize) {
    console.log(`  --  dist/${art.download}  ${localSize} bytes locally, ${asset.size} uploaded — not the same file`);
    fail(`local dist/${art.download} differs from the uploaded asset — re-check on the machine that built it`);
    return;
  }

  // `stapler validate` is the crisp binary test: no ticket, non-zero exit.
  const stapled = (path) => spawnSync("xcrun", ["stapler", "validate", path], { encoding: "utf8" }).status === 0;
  const report = (label, ok, why) => {
    console.log(`  ${ok ? "ok" : "--"}  ${label.padEnd(26)} ${ok ? "notarization ticket stapled" : "NO notarization ticket"}`);
    if (!ok) fail(why);
  };
  report(art.download, stapled(dmg), `${art.download} is not notarized — Gatekeeper will block the file people download`);

  // The .app the updater installs. Tauri writes it beside the bundle it was made from.
  const app = join(root, "src-tauri", "target", "universal-apple-darwin", "release", "bundle", "macos", "ChillSweep.app");
  if (existsSync(app)) {
    report("ChillSweep.app", stapled(app), "the packaged app is not notarized — the updater's replacement would be rejected");
  }

  // macOS 14+ ships Apple's own pre-distribution linter, which names the problem far better
  // than spctl does. Absent on older systems, where stapler alone has to do.
  const sys = spawnSync("syspolicy_check", ["distribution", dmg], { encoding: "utf8" });
  if (sys.error) {
    console.log("  ..  syspolicy_check          unavailable (macOS 14+) — relied on stapler alone");
    notes.push("syspolicy_check unavailable; stapler was the only notarization test");
  } else {
    const ok = sys.status === 0;
    console.log(`  ${ok ? "ok" : "--"}  ${"syspolicy_check".padEnd(26)} ${ok ? "ready for distribution" : (sys.stdout || sys.stderr || "").trim().split("\n")[0]}`);
    if (!ok) fail(`syspolicy_check says ${art.download} is not ready for distribution`);
  }
}

function tagArg() {
  const hit = process.argv.find((a) => a.startsWith("--tag="));
  return hit ? hit.slice(6).replace(/^v?/i, "v") : `v${version()}`;
}

async function main() {
  const tok = token();
  if (!tok) throw new Error("no GH_TOKEN (release.env or the environment).");
  const api = client(tok);

  const tag = tagArg();
  const release = (await listReleases(api)).find((r) => r.tag_name === tag);
  if (!release) throw new Error(`no release tagged ${tag} on ${OWNER}/${REPO}.`);

  const byName = new Map(release.assets.map((a) => [a.name, a]));
  // Draft assets need the token and Accept: octet-stream; browser_download_url 404s while it's a draft.
  const read = async (name) => {
    const res = await api(`/releases/assets/${byName.get(name).id}`, { headers: { Accept: "application/octet-stream" } });
    return res.ok ? res.text() : null;
  };

  console.log(`${tag}   ${release.draft ? "draft — not visible to users or the updater" : "PUBLISHED — live to users"}\n`);
  console.log("assets");
  for (const { name, note } of EXPECTED) {
    const a = byName.get(name);
    if (a) console.log(`  ok  ${name.padEnd(26)} ${((a.size / 1048576).toFixed(1) + " MB").padStart(9)}`);
    else {
      console.log(`  --  ${name.padEnd(26)} ${"MISSING".padStart(9)}   ${note}`);
      fail(`${name} missing`);
    }
  }
  for (const a of release.assets) if (!EXPECTED.some((e) => e.name === a.name)) console.log(`  ??  ${a.name}  (extra)`);

  // ── Update feed ───────────────────────────────────────────────────────────
  console.log("\nupdate feed");
  if (byName.has(FEED)) {
    const text = await read(FEED);
    let feed = null;
    // `read` returns null when the download fails, and `JSON.parse(null)` quietly evaluates
    // `JSON.parse("null")` → null, so without this the catch never fires, every assertion below
    // is skipped, and the script still prints "Ready to publish".
    if (text === null) {
      fail(`couldn't download ${FEED} from the release to check it — re-run, and don't publish until this passes`);
    } else {
      try {
        feed = JSON.parse(text);
      } catch {
        fail(`${FEED} is unreadable or not JSON`);
      }
      if (feed === null) fail(`${FEED} is empty`);
    }
    if (feed) {
      if (`v${feed.version}` !== tag) fail(`${FEED} says version ${feed.version}, release is ${tag}`);
      // Every platform's keys have to be present and point at that platform's own updater
      // artifact with a matching signature. A missing one usually means the other machine
      // hasn't published into this draft yet, which is worth saying rather than just failing.
      for (const art of Object.values(ARTIFACTS)) {
        const wantUrl = `https://github.com/${OWNER}/${REPO}/releases/download/${tag}/${art.updater}`;
        const sigName = `${art.updater}.sig`;
        let sig = null;
        if (byName.has(sigName)) {
          const raw = await read(sigName);
          // Same trap as above: a failed read would silently skip signature verification, which
          // is the one check standing between a bad upload and an updater that rejects everything.
          if (raw === null) fail(`couldn't download ${sigName} to check it against ${FEED}`);
          else sig = raw.trim();
        }
        for (const key of art.feedKeys) {
          const entry = feed.platforms?.[key];
          if (!entry) {
            console.log(`  --  ${key.padEnd(22)} missing`);
            fail(`${FEED} has no platforms["${key}"] entry — has the ${art.label} machine published into ${tag} yet?`);
            continue;
          }
          let bad = false;
          if (entry.url !== wantUrl) {
            fail(`${FEED}'s ${key} points at ${entry.url}, expected ${wantUrl}`);
            bad = true;
          }
          if (sig && entry.signature !== sig) {
            fail(`${FEED}'s ${key} signature doesn't match ${sigName} — the updater would reject it`);
            bad = true;
          }
          console.log(`  ${bad ? "--" : "ok"}  ${key.padEnd(22)} -> ${art.updater}`);
        }
      }
      console.log(`  ok  version ${feed.version}`);
    }
  } else {
    console.log("  --  not uploaded");
  }

  // ── macOS signing ─────────────────────────────────────────────────────────
  console.log("\nmacOS signing");
  checkMacSigning(byName);

  // ── Notes ─────────────────────────────────────────────────────────────────
  console.log("\nnotes");
  const body = release.body || "";
  const prose = whatsNewProse(body);
  if (!prose) {
    console.log("  --  What's new           absent");
    fail("What's new is missing — run npm run release-notes");
  } else if (prose === SEED) {
    console.log("  --  What's new           still the placeholder");
    fail("What's new is still the \"- …\" placeholder — write CHANGELOG.md and run npm run release-notes -- --refresh");
  } else {
    console.log(`  ok  What's new           ${prose.split("\n").filter((l) => l.trim()).length} line(s)`);
  }
  const s = body.indexOf(START), e = body.indexOf(END);
  const rows = s !== -1 && e !== -1 ? parseRows(body.slice(s, e)) : new Map();
  const uploaded = release.assets.map((a) => a.name).filter((n) => DOWNLOADABLE.test(n));
  const missingRows = uploaded.filter((n) => !rows.has(n));
  if (missingRows.length) {
    console.log(`  --  checksums            no row for: ${missingRows.join(", ")}`);
    fail(`no checksum row for ${missingRows.join(", ")} — run npm run release-notes`);
  } else {
    console.log(`  ok  checksums            covers all ${uploaded.length} download(s)`);
  }

  console.log();
  // Anything this run could not test, said out loud. Run from Windows, the macOS notarization
  // checks are skipped entirely — so an unqualified "Ready to publish" would be claiming more
  // than was actually verified.
  if (notes.length > 0) {
    console.log(`${notes.length} thing(s) this run could NOT check:`);
    for (const n of notes) console.log(`  ? ${n}`);
    console.log();
  }
  if (problems.length === 0) {
    const caveat = notes.length > 0 ? " Note the unchecked items above first." : "";
    console.log(
      release.draft
        ? `Ready to publish.${caveat} Publishing is what ships the update to every installed copy — do it by hand on the release page.`
        : `Published release looks complete.${caveat}`,
    );
    return;
  }
  console.log(`DO NOT PUBLISH — ${problems.length} problem(s):`);
  for (const p of problems) console.log(`  - ${p}`);
  process.exitCode = 1;
}

main().catch((err) => {
  console.error(`release-check failed: ${err.message}`);
  process.exitCode = 1;
});
