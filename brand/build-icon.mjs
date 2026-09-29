// Generates the ChillSweep icon set: an open fridge with leftovers on the shelves, drawn in Vocal
// Slice's style, a blue→mauve mark on a dark Catppuccin tile.
//
//   node brand/build-icon.mjs          → writes brand/icon.svg, brand/mark.svg and src/logo.svg,
//                                        then regenerates src-tauri/icons/* with `tauri icon`
//
// Geometry lives in one place so the tiled icon and the bare mark stay in step.

import { copyFileSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { execSync } from "node:child_process";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const BRAND = dirname(fileURLToPath(import.meta.url));

// Catppuccin Mocha, same values as Vocal Slice.
const C = { blue: "#89B4FA", mauve: "#CBA6F7", base: "#1E1E2E", baseHi: "#2A2A3D" };

const SIZE = 1024;
const RADIUS = 230; // ~22% corner, full bleed (Windows and the web want the art to fill the canvas)

// ── Fridge geometry (1024 canvas) ─────────────────────────────────────────────
const F = { x: 262, y: 160, w: 330, h: 704, r: 56 }; // body
const SPLIT = F.y + Math.round(F.h * 0.34); // freezer / fridge seam
const GAP = 22; // seam and cut-out width
const HANDLE = { x: F.x + 44, w: 30, r: 15 };

function mark(fill) {
  const top = SPLIT + GAP / 2;
  const bottom = F.y + F.h;
  const wall = 30;
  const inner = { x: F.x + wall, y: top + wall, w: F.w - wall * 2, h: bottom - top - wall * 2 };

  // Cut out of the body: the door seam, the freezer handle and the open cabinet.
  const cuts = [
    `<rect x="${F.x - 1}" y="${SPLIT - GAP / 2}" width="${F.w + 2}" height="${GAP}"/>`,
    `<rect x="${HANDLE.x}" y="${SPLIT - 150}" width="${HANDLE.w}" height="104" rx="${HANDLE.r}"/>`,
    `<rect x="${inner.x}" y="${inner.y}" width="${inner.w}" height="${inner.h}" rx="18"/>`,
  ];

  // Leftovers on two shelves, in the same gradient as the fridge.
  const shelfY = inner.y + Math.round(inner.h * 0.5);
  const contents = [
    `<rect x="${inner.x}" y="${shelfY - 8}" width="${inner.w}" height="16" rx="8"/>`, // shelf
    `<rect x="${inner.x + 34}" y="${shelfY - 8 - 118}" width="104" height="118" rx="20"/>`, // tall tub
    `<rect x="${inner.x + 158}" y="${shelfY - 8 - 74}" width="70" height="74" rx="16"/>`, // small box
    `<rect x="${inner.x + 34}" y="${inner.y + inner.h - 96}" width="150" height="96" rx="20"/>`, // takeaway box
  ];

  // The lower door swung open: a trapezoid hinged on the right edge, taller at its free edge
  // because that edge has turned towards us.
  const hx = F.x + F.w + 14;
  const dw = 132;
  const door =
    `<path d="M${hx} ${top} L${hx + dw} ${top - 34} Q${hx + dw + 14} ${top - 36} ${hx + dw + 14} ${top - 20} ` +
    `L${hx + dw + 14} ${bottom + 20} Q${hx + dw + 14} ${bottom + 36} ${hx + dw} ${bottom + 34} L${hx} ${bottom} Z"/>`;
  const doorHandle = `<rect x="${hx + dw - 42}" y="${top + 40}" width="${HANDLE.w - 4}" height="150" rx="${HANDLE.r - 2}"/>`;

  const cutMask = (id, shapes) =>
    `<mask id="${id}"><rect width="${SIZE}" height="${SIZE}" fill="#fff"/><g fill="#000">${shapes}</g></mask>`;
  return (
    cutMask("cut-body", cuts.join("")) +
    cutMask("cut-door", doorHandle) +
    `<g fill="${fill}">` +
    `<rect x="${F.x}" y="${F.y}" width="${F.w}" height="${F.h}" rx="${F.r}" mask="url(#cut-body)"/>` +
    contents.join("") +
    `<g mask="url(#cut-door)">${door}</g>` +
    `</g>`
  );
}

function svg(inner, defs = "") {
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${SIZE} ${SIZE}" width="${SIZE}" height="${SIZE}"><defs>${defs}</defs>${inner}</svg>\n`;
}

// The gradient spans the whole mark, not each shape, so every piece shares one sweep.
const defs =
  `<linearGradient id="g" gradientUnits="userSpaceOnUse" x1="${F.x}" y1="${F.y}" x2="${F.x + F.w + 160}" y2="${F.y + F.h}">` +
  `<stop offset="0" stop-color="${C.blue}"/><stop offset="1" stop-color="${C.mauve}"/></linearGradient>` +
  `<radialGradient id="tile" cx="0.3" cy="0.2" r="1"><stop offset="0" stop-color="${C.baseHi}"/><stop offset="1" stop-color="${C.base}"/></radialGradient>`;
const tile = `<rect width="${SIZE}" height="${SIZE}" rx="${RADIUS}" fill="url(#tile)"/>`;

writeFileSync(join(BRAND, "icon.svg"), svg(tile + mark("url(#g)"), defs));
writeFileSync(join(BRAND, "mark.svg"), svg(mark("currentColor")));
console.log("wrote brand/icon.svg, brand/mark.svg");

// ── App icons ─────────────────────────────────────────────────────────────────
const ROOT = join(BRAND, "..");
const ICONS = join(ROOT, "src-tauri", "icons");
copyFileSync(join(BRAND, "icon.svg"), join(ROOT, "src", "logo.svg"));

const tmp = mkdtempSync(join(tmpdir(), "chillsweep-icons-"));
try {
  execSync(`npx tauri icon "${join(BRAND, "icon.svg")}" -o "${tmp}"`, { cwd: ROOT, stdio: "ignore" });
  // Only the files tauri.conf.json and the Windows bundle use; skip the Android/iOS output.
  const names = ["32x32.png", "128x128.png", "128x128@2x.png", "icon.png", "icon.ico", "icon.icns", "StoreLogo.png"];
  for (const s of [30, 44, 71, 89, 107, 142, 150, 284, 310]) names.push(`Square${s}x${s}Logo.png`);
  for (const name of names) copyFileSync(join(tmp, name), join(ICONS, name));
} finally {
  rmSync(tmp, { recursive: true, force: true });
}
console.log("wrote src/logo.svg and src-tauri/icons/*");
