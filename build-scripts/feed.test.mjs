// Tests for the shared update feed: `node --test build-scripts/` (or `npm test`).
//
// Worth testing when nothing else here is, because this is the one piece whose failure is
// SILENT. A release with a clobbered `latest.json` still looks fine — the download works, the
// page looks right, `release:check` is the only thing that would notice — and the symptom is
// just that one platform stops being offered updates, possibly for months.

import assert from "node:assert/strict";
import { test } from "node:test";
import { ARTIFACTS, mergeFeed } from "./github.mjs";

const meta = { version: "1.11.0", notes: "- did a thing", pubDate: "2026-10-02T00:00:00.000Z" };
const win = { "windows-x86_64": { signature: "sig-win", url: "https://example/ChillSweep-Setup.exe" } };
const mac = {
  "darwin-universal": { signature: "sig-mac", url: "https://example/u.app.tar.gz" },
  "darwin-aarch64": { signature: "sig-mac", url: "https://example/u.app.tar.gz" },
  "darwin-x86_64": { signature: "sig-mac", url: "https://example/u.app.tar.gz" },
};

test("the first machine writes a feed from nothing", () => {
  const feed = mergeFeed(null, win, meta);
  assert.equal(feed.version, "1.11.0");
  assert.equal(feed.notes, "- did a thing");
  assert.deepEqual(Object.keys(feed.platforms), ["windows-x86_64"]);
});

test("the second machine adds to the first instead of replacing it", () => {
  const first = mergeFeed(null, win, meta);
  const second = mergeFeed(first, mac, meta);
  // The whole point: Windows is still there after macOS publishes.
  assert.deepEqual(Object.keys(second.platforms).sort(), ["darwin-aarch64", "darwin-universal", "darwin-x86_64", "windows-x86_64"]);
  assert.equal(second.platforms["windows-x86_64"].signature, "sig-win");
  assert.equal(second.platforms["darwin-universal"].signature, "sig-mac");
});

test("order doesn't matter", () => {
  const a = mergeFeed(mergeFeed(null, win, meta), mac, meta);
  const b = mergeFeed(mergeFeed(null, mac, meta), win, meta);
  assert.deepEqual(Object.keys(a.platforms).sort(), Object.keys(b.platforms).sort());
});

test("re-running the same machine replaces its own entries, not the other's", () => {
  const both = mergeFeed(mergeFeed(null, win, meta), mac, meta);
  const rerun = mergeFeed(both, { "windows-x86_64": { signature: "sig-win-2", url: "https://example/new.exe" } }, meta);
  assert.equal(rerun.platforms["windows-x86_64"].signature, "sig-win-2");
  assert.equal(rerun.platforms["darwin-universal"].signature, "sig-mac", "macOS must survive a Windows re-run");
});

test("a version mismatch is refused, because it means the machines diverged", () => {
  const first = mergeFeed(null, win, meta);
  assert.throws(() => mergeFeed(first, mac, { ...meta, version: "1.12.0" }), /different commits/);
});

test("the notes and date of whoever got there first are kept", () => {
  const first = mergeFeed(null, win, meta);
  const second = mergeFeed(first, mac, { ...meta, notes: "- something else", pubDate: "2026-12-25T00:00:00.000Z" });
  assert.equal(second.notes, "- did a thing", "the second run must not rewrite the release notes");
  assert.equal(second.pub_date, meta.pubDate);
});

test("every platform in the manifest declares at least one feed key", () => {
  for (const [name, art] of Object.entries(ARTIFACTS)) {
    assert.ok(art.feedKeys.length > 0, `${name} has no feedKeys`);
    assert.ok(art.download && art.updater, `${name} is missing an artifact name`);
  }
  // A key claimed by two platforms would have them overwrite each other on every release.
  const all = Object.values(ARTIFACTS).flatMap((a) => a.feedKeys);
  assert.equal(new Set(all).size, all.length, "two platforms claim the same feed key");
});
