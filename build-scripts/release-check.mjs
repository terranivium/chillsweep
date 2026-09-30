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

import { client, FEED, INSTALLER, listReleases, OWNER, REPO, SIGNATURE, token } from "./github.mjs";
import { DOWNLOADABLE, END, parseRows, SEED, START, whatsNewProse } from "./release-notes.mjs";
import { version } from "./version.mjs";

const EXPECTED = [
  { name: INSTALLER, note: "the installer people download" },
  { name: SIGNATURE, note: "the updater refuses an installer without it" },
  { name: FEED, note: "the update feed installed copies read" },
];

const problems = [];
const fail = (m) => problems.push(m);

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
    try {
      feed = JSON.parse(text);
    } catch {
      fail(`${FEED} is unreadable or not JSON`);
    }
    const win = feed?.platforms?.["windows-x86_64"];
    const wantUrl = `https://github.com/${OWNER}/${REPO}/releases/download/${tag}/${INSTALLER}`;
    const before = problems.length;
    if (feed) {
      if (`v${feed.version}` !== tag) fail(`${FEED} says version ${feed.version}, release is ${tag}`);
      if (!win) fail(`${FEED} has no platforms["windows-x86_64"] entry`);
      else if (win.url !== wantUrl) fail(`${FEED} points at ${win.url}, expected ${wantUrl}`);
      if (win && byName.has(SIGNATURE)) {
        const sig = (await read(SIGNATURE))?.trim();
        if (sig !== win.signature) fail(`${FEED}'s signature doesn't match ${SIGNATURE} — the updater would reject it`);
      }
      console.log(`  ${problems.length > before ? "--" : "ok"}  version ${feed.version} -> ${win?.url ?? "?"}`);
    }
  } else {
    console.log("  --  not uploaded");
  }

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
  if (problems.length === 0) {
    console.log(
      release.draft
        ? "Ready to publish. Publishing is what ships the update to every installed copy — do it by hand on the release page."
        : "Published release looks complete.",
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
