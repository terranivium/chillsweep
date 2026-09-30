// Reads CHANGELOG.md, the single source of truth for every release's notes (see RELEASING.md).
// Nothing here writes it. The GitHub release body (release-notes.mjs) and the in-app "What's new"
// (changelog-json.mjs → src/changelog.json) both come through here, so they can't drift apart.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { root } from "./version.mjs";

export const changelogPath = () => join(root, "CHANGELOG.md");

export function readChangelog() {
  try {
    return readFileSync(changelogPath(), "utf8");
  } catch {
    return null;
  }
}

// A section heading: "## Unreleased", or "## 1.421.0" with an optional " — 2026-07-22" suffix.
// A version must be followed by a boundary so "1.42" can't match "## 1.421.0".
export function headingRe(name) {
  return name.toLowerCase() === "unreleased"
    ? /^##[ \t]+Unreleased[ \t]*$/im
    : new RegExp(`^##[ \\t]+${name.replace(/\./g, "\\.")}(?![\\d.])[^\\n]*$`, "m");
}

/**
 * One section's body, trimmed, or null when absent or empty. `name` is "Unreleased" or a version.
 *
 * While a release is in flight its notes live under "## Unreleased", not a version heading: the
 * version is 1.{commit count}.0, so the commit that renamed the heading would itself bump the count
 * and make the heading wrong by one. `npm run changelog:promote` renames it AFTER publishing.
 */
export function changelogSection(name, text = readChangelog()) {
  if (!text) return null;
  const h = text.match(headingRe(name));
  if (!h) return null;
  const rest = text.slice(h.index + h[0].length);
  const next = rest.search(/^##[ \t]+/m);
  return (next === -1 ? rest : rest.slice(0, next)).trim() || null;
}

// "## 1.451.0 — 2026-08-11". Em dash, matching what --promote writes.
const VERSION_HEADING = /^##[ \t]+(\d+\.\d+\.\d+)[ \t]+—[ \t]+(\d{4}-\d{2}-\d{2})[ \t]*$/gm;

// A section is prose paragraphs and/or bullets. Keep both, in order.
export function parseBody(body) {
  const blocks = [];
  for (const raw of body.split(/\r?\n(?=[ \t]*-[ \t])|(?:\r?\n){2,}/)) {
    const text = raw.trim();
    if (!text) continue;
    if (/^-[ \t]/.test(text)) {
      // Continuation lines of a wrapped bullet are indented; fold them back into one line.
      const item = text.replace(/^-[ \t]+/, "").replace(/\s*\r?\n\s*/g, " ").trim();
      if (blocks.length && blocks.at(-1).type === "list") blocks.at(-1).items.push(item);
      else blocks.push({ type: "list", items: [item] });
    } else {
      blocks.push({ type: "text", text: text.replace(/\s*\r?\n\s*/g, " ") });
    }
  }
  return blocks;
}

// Numeric, not lexicographic: "1.9.0" must not sort above "1.451.0".
export function cmpVersion(a, b) {
  const pa = a.split(".").map(Number), pb = b.split(".").map(Number);
  for (let i = 0; i < 3; i++) if (pa[i] !== pb[i]) return pb[i] - pa[i];
  return 0;
}

/**
 * Every shipped release, newest first.
 *
 * `unreleasedAs` labels "## Unreleased" as the version being built. Promotion happens after
 * publishing, so at build time Unreleased IS the notes for this build; without it the app would ship
 * a "What's new" that stops one release short of itself.
 */
export function releases({ unreleasedAs = null } = {}) {
  const text = readChangelog() || "";
  const out = [];
  for (const [, version, date] of text.matchAll(VERSION_HEADING)) {
    const body = changelogSection(version, text);
    if (body) out.push({ version, date, blocks: parseBody(body) });
  }
  if (unreleasedAs && !out.some((r) => r.version === unreleasedAs.version)) {
    const body = changelogSection("Unreleased", text);
    if (body) out.push({ ...unreleasedAs, blocks: parseBody(body) });
  }
  return out.sort((a, b) => (a.date < b.date ? 1 : a.date > b.date ? -1 : cmpVersion(a.version, b.version)));
}

// ── Inline markdown ──────────────────────────────────────────────────────────
// Entries use exactly three inline constructs: `code`, **bold** and *italic*. Tokenised here, once,
// so the app renders DOM nodes from segments instead of showing raw asterisks. Deliberately not a
// markdown parser: an unmatched marker stays literal text. Backticks match first, so an asterisk
// inside `a*b` is never mistaken for emphasis.
const INLINE_RE = /`([^`]+)`|\*\*([^*]+)\*\*|\*([^*\s][^*]*)\*/g;

export function inlineSegments(text) {
  const str = String(text ?? "");
  const out = [];
  let last = 0;
  for (const m of str.matchAll(INLINE_RE)) {
    if (m.index > last) out.push({ type: "text", value: str.slice(last, m.index) });
    if (m[1] !== undefined) out.push({ type: "code", value: m[1] });
    else if (m[2] !== undefined) out.push({ type: "strong", value: m[2] });
    else out.push({ type: "em", value: m[3] });
    last = m.index + m[0].length;
  }
  if (last < str.length) out.push({ type: "text", value: str.slice(last) });
  return out;
}

/** The same blocks with every string replaced by its segment list, for src/changelog.json. */
export function segmentBlocks(blocks) {
  return blocks.map((b) =>
    b.type === "list" ? { ...b, items: b.items.map(inlineSegments) } : { ...b, text: inlineSegments(b.text) },
  );
}
