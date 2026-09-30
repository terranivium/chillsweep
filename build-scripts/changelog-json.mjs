// Bakes CHANGELOG.md into src/changelog.json so About → What's new works offline.
//
// Runs as tauri.conf.json's beforeDevCommand and beforeBuildCommand, so every build and every
// `tauri dev` gets a fresh copy. The file is generated, so it's gitignored.

import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { releases, segmentBlocks } from "./changelog-data.mjs";
import { buildNumber, root, version } from "./version.mjs";

// "## Unreleased" is the notes for the version being built (see releases()). Only label it when there
// is a real build number; without git there's no honest version to put on it.
const unreleasedAs = buildNumber() ? { version: version(), date: new Date().toISOString().slice(0, 10) } : null;

const list = releases({ unreleasedAs }).map((r) => ({ ...r, blocks: segmentBlocks(r.blocks) }));
writeFileSync(join(root, "src", "changelog.json"), JSON.stringify(list, null, 2) + "\n");
console.log(`changelog.json: ${list.length} release(s)${unreleasedAs ? `, Unreleased as ${unreleasedAs.version}` : ""}`);
