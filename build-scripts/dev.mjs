// Runs the app in development with the real version baked in.
//
//   node build-scripts/dev.mjs      (npm run dev)
//
// tauri.conf.json keeps a static "1.0.0", and `tauri dev` on its own reports exactly that — so the
// About screen used to read v1.0.0 during development while a release of the same commit called
// itself 1.21.0. This passes the same `--config` override pack.mjs does, from the same `version()`,
// so About tells the truth in every build and there is still one place the scheme lives.
//
// Any extra arguments are handed to `tauri dev` untouched.

import { spawnSync } from "node:child_process";
import { join } from "node:path";
import { root, version } from "./version.mjs";

const config = { version: version() };
console.log(`dev: running ChillSweep ${config.version}`);

const cli = join(root, "node_modules", "@tauri-apps", "cli", "tauri.js");
const args = ["dev", "--config", JSON.stringify(config), ...process.argv.slice(2)];
const result = spawnSync(process.execPath, [cli, ...args], { stdio: "inherit", cwd: root });
process.exit(result.status ?? 1);
