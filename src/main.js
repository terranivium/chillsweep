const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { revealItemInDir } = window.__TAURI__.opener;

// What this OS calls things. Filled in from `app_info` before the first render, so no part of
// the page has to guess the platform. The defaults keep the UI readable if that call ever fails.
const OS = { platform: "", binName: "the Recycle Bin or Trash", fileManager: "your file manager" };

const TIERS = {
  safe: { icon: "ph-broom", label: "Safe to clear", blurb: "Caches and junk that rebuild themselves." },
  leftover: { icon: "ph-package", label: "Leftovers", blurb: "Left behind by apps and games that are no longer installed." },
  your_call: { icon: "ph-question", label: "Your call", blurb: "Removable, but only you know if you still want these." },
};

const CATEGORY_LABELS = {
  cache: "Cache",
  leftover: "Leftover",
  temp: "Temp",
  stray: "Stray",
  dev: "Dev",
  games: "Games",
  downloads: "Downloads",
  projects: "Projects",
};

const $ = (sel) => document.querySelector(sel);

/** The report on screen. Kept so the page can keep its totals in step as rows go. */
let shown = null;
/** Findings from the current report, by id. */
let findings = new Map();
/** Ids of findings the user ticked. */
const selected = new Set();
/** First error seen per finding during the clean in progress, for the row to show. */
const cleanErrors = new Map();

function formatSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 100 ? v.toFixed(0) : v.toFixed(1)} ${units[i]}`;
}

function plural(n, word) {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function setStatus(text) {
  const status = $("#status");
  status.textContent = text;
  status.hidden = !text;
}

// ---------------------------------------------------------------------------
// The status bar: what's selected when idle, live progress while working
// ---------------------------------------------------------------------------

/** Paths kept in the feed. Enough to read motion from, few enough to stay out of the way. */
const FEED_LINES = 3;

// The backend throttles to about sixteen updates a second; this folds whatever arrives into one
// write per frame, so the bar and the feed never thrash layout.
let pendingScan = null;
let pendingClean = null;
let frame = null;

function setMode(mode) {
  $("#statusbar").dataset.mode = mode;
  document.body.classList.toggle("is-working", mode !== "idle");
}

function setBar(percent) {
  const pct = Math.max(0, Math.min(100, percent));
  // Through the style object, not a style attribute: the page's CSP has no 'unsafe-inline', and
  // CSSOM isn't subject to it.
  $("#sb-fill").style.width = `${pct}%`;
  $("#sb-bar").setAttribute("aria-valuenow", Math.round(pct));
}

function feedPush(path) {
  const feed = $("#sb-feed");
  feed.append(el("span", null, path));
  while (feed.childElementCount > FEED_LINES) feed.firstElementChild.remove();
}

function resetProgress(stage) {
  pendingScan = null;
  pendingClean = null;
  $("#sb-feed").replaceChildren();
  $("#sb-stage").textContent = stage;
  $("#sb-count").textContent = "";
  setBar(0);
}

function schedule() {
  if (frame) return;
  frame = requestAnimationFrame(() => {
    frame = null;
    if (pendingScan) {
      paintScan(pendingScan);
      pendingScan = null;
    }
    if (pendingClean) {
      paintClean(pendingClean);
      pendingClean = null;
    }
  });
}

function paintScan(p) {
  setBar(p.percent);
  $("#sb-stage").textContent = p.label;
  const found = p.found > 0 ? ` · ${plural(p.found, "item")} so far, ${formatSize(p.bytes)}` : "";
  $("#sb-count").textContent = `Step ${p.step} of ${p.steps}${found}`;
  if (p.path) feedPush(p.path);
}

function paintClean(p) {
  setBar(p.total > 0 ? (p.done / p.total) * 100 : 0);
  $("#sb-stage").textContent = p.outcome.title ? `Removing ${p.outcome.title}` : "Removing…";
  const gone = p.freed_bytes + p.recycled_bytes;
  $("#sb-count").textContent = `${p.done} of ${p.total}${gone > 0 ? ` · ${formatSize(gone)}` : ""}`;
  if (p.outcome.path) feedPush(p.outcome.path);
}

/// Each item as it finishes. The bar and feed are coalesced; retiring a row is not, because
/// missing one would leave something on screen that is no longer there.
function onCleanProgress(p) {
  pendingClean = p;
  schedule();
  const id = p.outcome.finding_id;
  if (p.outcome.error && !cleanErrors.has(id)) cleanErrors.set(id, p.outcome.error);
  if (!p.finding_done) return;
  if (p.finding_failed) markRowFailed(id, cleanErrors.get(id));
  else retireRow(id);
}

// ---------------------------------------------------------------------------
// Report rendering
// ---------------------------------------------------------------------------

function renderSummary(report) {
  const summary = $("#summary");
  summary.replaceChildren();
  for (const total of report.totals) {
    const tier = TIERS[total.tier];
    const card = el("a", `card tier-${total.tier}`);
    card.href = `#tier-${total.tier}`;
    const label = el("span", "card-label");
    label.append(el("i", `ph ${tier.icon}`), document.createTextNode(tier.label));
    card.append(label,el("span", "card-size", formatSize(total.bytes)), el("span", "card-count", plural(total.count, "item")));
    if (total.count === 0) card.classList.add("empty");
    summary.append(card);
  }
  summary.hidden = false;
}

/// Project findings sit outside the tiers, so each says which tier it would be in instead of
/// repeating "Projects".
function renderFinding(f, { inProjects = false } = {}) {
  const node = $("#finding-template").content.firstElementChild.cloneNode(true);
  node.dataset.id = f.id;
  node.querySelector(".title").textContent = f.title;
  node.querySelector(".size").textContent = formatSize(f.bytes);

  const pick = node.querySelector(".pick");
  pick.setAttribute("aria-label", `Select ${f.title}`);
  pick.addEventListener("change", () => {
    if (pick.checked) selected.add(f.id);
    else selected.delete(f.id);
    updateSelectionBar();
  });

  const chips = node.querySelector(".chips");
  if (inProjects) chips.append(el("span", `chip tier-chip tier-${f.tier}`, TIERS[f.tier].label));
  else chips.append(el("span", "chip", CATEGORY_LABELS[f.category] ?? f.category));
  if (f.confidence !== "high") chips.append(el("span", `chip conf-${f.confidence}`, `${f.confidence} confidence`));

  const what = node.querySelector(".what");
  if (f.what) what.textContent = f.what;
  else what.remove();

  const evidence = node.querySelector(".evidence");
  for (const e of f.evidence) evidence.append(el("li", null, e));

  const ifDeleted = node.querySelector(".if-deleted");
  if (f.if_deleted) ifDeleted.append(el("strong", null, "If you remove it: "), document.createTextNode(f.if_deleted));
  else ifDeleted.remove();

  fillItems(node, f);
  return node;
}

/// The paths a finding covers. Redrawn rather than patched, because a clean-up can take some of a
/// finding's paths and leave others: a row that survived would otherwise still list what has gone,
/// with a "Show in …" button pointing at a path that is no longer there.
function fillItems(node, f) {
  const items = node.querySelector(".items");
  items.replaceChildren();
  for (const it of f.items) {
    const li = el("li", "item");
    const path = el("span", "path", it.path);
    path.title = it.path;
    const meta = el("span", "item-size", it.is_dir ? `${formatSize(it.bytes)} · ${it.files.toLocaleString()} files` : formatSize(it.bytes));
    const reveal = el("button", "link", `Show in ${OS.fileManager}`);
    reveal.type = "button";
    reveal.addEventListener("click", () => revealItemInDir(it.path).catch((e) => setStatus(`Couldn't open ${OS.fileManager}: ${e}`)));
    li.append(path, meta, reveal);
    items.append(li);
  }
}

function renderResults(report) {
  const results = $("#results");
  results.replaceChildren();
  for (const total of report.totals) {
    if (total.count === 0) continue;
    const tier = TIERS[total.tier];
    const tierFindings = report.findings.filter((f) => f.tier === total.tier && f.category !== "projects");
    const section = el("section", `tier tier-${total.tier}`);
    section.id = `tier-${total.tier}`;

    const head = el("div", "tier-head");
    head.append(el("i", `tier-icon ph ${tier.icon}`), el("h2", null, tier.label), el("span", "tier-total", formatSize(total.bytes)));
    const all = el("button", "link select-all", "Select all");
    all.type = "button";
    all.addEventListener("click", () => {
      // Off the live rows, not the list captured when this was rendered: rows retire as a
      // clean-up confirms them, and a stale id here would be sent to the next clean and come
      // back as "Not in the last scan".
      const ids = [...section.querySelectorAll(".finding")].map((n) => n.dataset.id);
      const allSelected = ids.every((id) => selected.has(id));
      for (const id of ids) allSelected ? selected.delete(id) : selected.add(id);
      syncCheckboxes();
      updateSelectionBar();
    });
    head.append(all);
    section.append(head, el("p", "tier-blurb", tier.blurb));

    const list = el("div", "findings");
    for (const f of tierFindings) list.append(renderFinding(f));
    section.append(list);
    results.append(section);
  }
  renderProjects(report, results);
}

/// Project findings in their own section, collapsed and left out of the headline total.
///
/// They matter only to people who use those tools, and the tiers above are the general clean-up.
/// No Select all here: this section can hold whole projects, which should each be a deliberate pick.
function renderProjects(report, results) {
  if (!report.projects || report.projects.count === 0) return;
  const projectFindings = report.findings.filter((f) => f.category === "projects").sort((a, b) => b.bytes - a.bytes);
  const section = el("details", "projects");
  section.id = "projects";
  const head = el("summary", "tier-head");
  head.append(
    el("i", "chev ph ph-caret-right"),
    el("i", "tier-icon ph ph-folder-open"),
    el("h2", null, "Projects"),
    el("span", "tier-total", formatSize(report.projects.bytes)),
  );
  section.append(head, el("p", "tier-blurb", "Caches and old work from creative tools and game engines, found by their project files."));
  const list = el("div", "findings");
  for (const f of projectFindings) list.append(renderFinding(f, { inProjects: true }));
  section.append(list);
  results.append(section);
}

/// Totals as `scan.rs` computes them: project findings counted apart from the three tiers.
///
/// Worked out here as well as in Rust so the headline numbers can follow rows off the screen
/// during a clean, instead of contradicting the list for a few seconds.
function computeTotals(list) {
  const general = list.filter((f) => f.category !== "projects");
  const totals = ["safe", "leftover", "your_call"].map((tier) => {
    const of = general.filter((f) => f.tier === tier);
    return { tier, bytes: of.reduce((sum, f) => sum + f.bytes, 0), count: of.length };
  });
  const projects = list.filter((f) => f.category === "projects");
  return { totals, projects: { bytes: projects.reduce((sum, f) => sum + f.bytes, 0), count: projects.length } };
}

function refreshTotals(current) {
  const { totals, projects } = computeTotals(current.findings);
  current.totals = totals;
  current.projects = projects;
  renderSummary(current);
  for (const total of totals) {
    const node = document.querySelector(`#tier-${total.tier} .tier-total`);
    if (node) node.textContent = formatSize(total.bytes);
  }
  const node = document.querySelector("#projects .tier-total");
  if (node) node.textContent = formatSize(projects.bytes);
}

function row(id) {
  return document.querySelector(`.finding[data-id="${CSS.escape(id)}"]`);
}

/// A tier or the projects section with nothing left in it has nothing to say.
function pruneSection(list) {
  if (!list || list.childElementCount > 0) return;
  list.closest(".tier, .projects")?.remove();
}

/// This finding is gone: out of the page's model, out of the totals, and off the screen.
function retireRow(id) {
  findings.delete(id);
  selected.delete(id);
  if (shown) {
    shown.findings = shown.findings.filter((f) => f.id !== id);
    refreshTotals(shown);
  }
  updateSelectionBar();
  const node = row(id);
  if (!node) return;
  // A concrete height to start from: there is nothing for max-height to animate out of otherwise.
  node.style.maxHeight = `${node.offsetHeight}px`;
  void node.offsetHeight;
  node.classList.add("removing");
  node.style.maxHeight = "0px";
  const finish = () => {
    if (!node.isConnected) return;
    const list = node.parentElement;
    node.remove();
    pruneSection(list);
  };
  node.addEventListener("transitionend", finish, { once: true });
  // Reduced motion turns the transition off, so transitionend never comes.
  setTimeout(finish, 400);
}

/// Something in this finding was left behind, so the row stays and says why.
function markRowFailed(id, reason) {
  selected.delete(id);
  updateSelectionBar();
  const node = row(id);
  if (!node) return;
  node.classList.add("failed");
  node.querySelector(".pick").checked = false;
  if (!node.querySelector(".row-problem")) {
    node.append(el("p", "row-problem", reason ?? "Some of this couldn't be removed."));
  }
}

/// Take the report the backend just handed us as the truth, without rebuilding the list.
///
/// Rows have already left as their items were confirmed, so re-rendering would only throw away
/// whatever the user had expanded and where they were scrolled. This corrects the sizes of what
/// survived and drops anything an event didn't account for.
function syncRows(current) {
  shown = current;
  findings = new Map(current.findings.map((f) => [f.id, f]));
  for (const node of document.querySelectorAll(".finding")) {
    const f = findings.get(node.dataset.id);
    if (!f) {
      retireRow(node.dataset.id);
    } else {
      node.querySelector(".size").textContent = formatSize(f.bytes);
      fillItems(node, f);
    }
  }
  for (const id of [...selected]) if (!findings.has(id)) selected.delete(id);
  refreshTotals(current);
  updateSelectionBar();
}

function adoptReport(current) {
  shown = current;
  findings = new Map(current.findings.map((f) => [f.id, f]));
  renderSummary(current);
  renderResults(current);
  renderFooter(current);
}

function renderFooter(report) {
  const inv = report.inventory;
  const footer = $("#footer");
  footer.replaceChildren(
    el(
      "p",
      null,
      `${inv.summary_text} in ${(report.duration_ms / 1000).toFixed(1)}s.`,
    ),
    ...report.warnings.map((w) => el("p", "muted", w)),
  );
  footer.hidden = false;
}

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

function syncCheckboxes() {
  for (const node of document.querySelectorAll(".finding")) {
    node.querySelector(".pick").checked = selected.has(node.dataset.id);
  }
  for (const section of document.querySelectorAll(".tier")) {
    const button = section.querySelector(".select-all");
    if (!button) continue;
    const ids = [...section.querySelectorAll(".finding")].map((n) => n.dataset.id);
    button.textContent = ids.every((id) => selected.has(id)) ? "Select none" : "Select all";
  }
}

function selectedFindings() {
  return [...selected].map((id) => findings.get(id)).filter(Boolean);
}

function updateSelectionBar() {
  const chosen = selectedFindings();
  const bytes = chosen.reduce((sum, f) => sum + f.bytes, 0);
  // The bar never leaves, so with nothing ticked it says what to do instead of "0 items".
  $("#statusbar").classList.toggle("empty", chosen.length === 0);
  $("#selection-text").textContent = chosen.length
    ? `${plural(chosen.length, "item")} selected · ${formatSize(bytes)}`
    : shown
      ? "Nothing selected. Tick what you want to remove."
      : "Scan to see what can be cleared.";
  $("#clean").disabled = chosen.length === 0;
  syncCheckboxes();
}

// ---------------------------------------------------------------------------
// Scan, clean, undo
// ---------------------------------------------------------------------------

async function scan({ keepOutcome = false } = {}) {
  const button = $("#scan");
  const label = button.querySelector(".label");
  button.disabled = true;
  label.textContent = "Scanning…";
  if (!keepOutcome) $("#outcome").hidden = true;
  setStatus("Scanning your folders. This only reads; nothing is changed.");
  resetProgress("Starting the scan…");
  setMode("scanning");
  try {
    const found = await invoke("scan");
    selected.clear();
    $("#intro").hidden = true;
    adoptReport(found);
    const reclaimable = found.totals.reduce((sum, t) => sum + t.bytes, 0);
    setStatus(`Found ${formatSize(reclaimable)} you could reclaim. Open any item to see why it was flagged, then tick what you want to remove.`);
  } catch (e) {
    setStatus(`Scan failed: ${e}`);
  } finally {
    setMode("idle");
    updateSelectionBar();
    button.disabled = false;
    label.textContent = "Scan again";
  }
}

function confirmClean() {
  const permanent = $("#permanent").checked;
  const chosen = selectedFindings();
  const groups = [
    { label: "Deleted permanently", note: "Frees space now. Can't be undone.", items: chosen.filter((f) => permanent && f.tier === "safe") },
    { label: `Moved to the ${OS.binName}`, note: `You can put these back from the ${OS.binName} afterwards.`, items: chosen.filter((f) => !(permanent && f.tier === "safe")) },
  ];
  const container = $("#confirm-groups");
  container.replaceChildren();
  for (const g of groups) {
    if (g.items.length === 0) continue;
    const bytes = g.items.reduce((sum, f) => sum + f.bytes, 0);
    const block = el("div", "confirm-group");
    block.append(el("h3", null, `${g.label}: ${formatSize(bytes)}`), el("p", "muted", g.note));
    const ul = el("ul");
    for (const f of g.items) ul.append(el("li", null, `${f.title} (${formatSize(f.bytes)})`));
    block.append(ul);
    container.append(block);
  }
  $("#confirm").showModal();
}

/// No scan afterwards: the backend hands back the report its own clean brought up to date, and
/// the rows have already gone one by one as each was confirmed. The scan this used to do ran with
/// the stale list still on screen, which made a finished clean-up look like it had done nothing.
async function runClean() {
  $("#confirm").close();
  const ids = [...selected];
  const permanentSafe = $("#permanent").checked;
  cleanErrors.clear();
  resetProgress(`Removing ${plural(ids.length, "item")}…`);
  setMode("cleaning");
  // Nothing else can be started while this runs: the Clean up button is behind the progress face,
  // and a scan would pull the list out from under it.
  $("#scan").disabled = true;
  try {
    const { result, report: updated } = await invoke("clean", { findingIds: ids, permanentSafe });
    showOutcome(result);
    syncRows(updated);
  } catch (e) {
    setStatus(`Clean-up failed: ${e}. Scan again before trying once more.`);
  } finally {
    $("#scan").disabled = false;
    setMode("idle");
    updateSelectionBar();
  }
}

/// Teach the page what this OS calls things, and fix up the sentences written in the HTML.
function applyPlatform(info) {
  OS.platform = info.platform ?? "";
  OS.binName = info.bin_name ?? OS.binName;
  OS.fileManager = info.file_manager ?? OS.fileManager;
  if (OS.platform) document.documentElement.dataset.platform = OS.platform;
  const inventory = OS.platform === "windows" ? "your folders and the registry" : "your folders and the apps you have installed";
  setText("about-scan", `A scan only reads ${inventory}; it never changes anything or starts other programs.`);
  setText(
    "about-clean",
    `When you clean up, items go to the ${OS.binName}, except safe-to-clear caches if you choose to delete those permanently.`,
  );
  const toggle = $("#permanent")?.closest(".toggle");
  if (toggle) toggle.title = `Safe-to-clear items rebuild themselves, so they skip the ${OS.binName}. Everything else always goes to the ${OS.binName}.`;
}

function setText(id, text) {
  const node = document.getElementById(id);
  if (node) node.textContent = text;
}

function showOutcome(result) {
  const parts = [];
  if (result.freed_bytes > 0) parts.push(`Freed ${formatSize(result.freed_bytes)}.`);
  if (result.recycled_bytes > 0) parts.push(`Moved ${formatSize(result.recycled_bytes)} to the ${OS.binName} (empty it to free the space).`);
  if (parts.length === 0) parts.push("Nothing was removed.");
  $("#outcome-text").textContent = parts.join(" ");

  const problems = $("#outcome-problems");
  problems.replaceChildren();
  for (const o of result.outcomes.filter((o) => o.error)) {
    const li = el("li");
    li.append(el("strong", null, o.title || o.finding_id), document.createTextNode(` · ${o.path}: ${o.error}`));
    problems.append(li);
  }

  // `recycle_bin` is the wire value of the method on both platforms; only the label differs.
  const recycled = result.outcomes.filter((o) => o.method === "recycle_bin" && !o.error).length;
  const undo = $("#undo");
  const reveal = $("#reveal-bin");
  // Where the backend gave us somewhere to reveal, it is telling us it cannot undo. Offer the
  // user the next best thing instead of a button that would fail.
  const canUndo = recycled > 0 && !result.reveal_dir;
  undo.hidden = !canUndo;
  undo.disabled = false;
  if (canUndo) undo.querySelector(".label").textContent = `Undo (put back ${plural(recycled, "item")})`;
  reveal.hidden = !(recycled > 0 && result.reveal_dir);
  reveal.textContent = `Show in ${OS.binName}`;
  reveal.onclick = () => revealItemInDir(result.reveal_dir).catch((e) => setStatus(`Couldn't open ${OS.fileManager}: ${e}`));
  $("#outcome").hidden = false;
}

async function undoLast() {
  const undo = $("#undo");
  undo.disabled = true;
  try {
    const result = await invoke("undo_last");
    const problems = $("#outcome-problems");
    problems.replaceChildren(...result.failed.map((f) => el("li", null, `${f.path}: ${f.error}`)));
    $("#outcome-text").textContent = `Put back ${plural(result.restored.length, "item")} from the ${OS.binName}.`;
    undo.hidden = true;
  } catch (e) {
    setStatus(`Undo failed: ${e}`);
    undo.disabled = false;
    return;
  }
  await scan({ keepOutcome: true });
}

// ---------------------------------------------------------------------------
// About
// ---------------------------------------------------------------------------

let aboutFilled = false;

async function openAbout({ whatsNew = false } = {}) {
  $("#about").showModal();
  if (whatsNew) $("#whats-new-heading").scrollIntoView({ block: "start" });
  if (aboutFilled) return;
  renderWhatsNew();
  try {
    const info = await invoke("app_info");
    $("#about-version").textContent = `v${info.version}`;
    $("#about-hash").textContent = info.git_hash;
    $("#about-built").textContent = info.build_time ? new Date(info.build_time * 1000).toLocaleString() : "—";
    aboutFilled = true;
  } catch (e) {
    $("#about-version").textContent = `Unavailable (${e})`;
  }
}

// ---------------------------------------------------------------------------
// What's new: CHANGELOG.md, baked into changelog.json at build time
// ---------------------------------------------------------------------------

/** Inline segments ({type, value}) from build-scripts/changelog-data.mjs, as DOM nodes. */
function segmentNodes(segments) {
  const tags = { code: "code", strong: "strong", em: "em" };
  return segments.map((s) => (tags[s.type] ? el(tags[s.type], null, s.value) : document.createTextNode(s.value)));
}

function humanDate(iso) {
  // Explicit UTC: a bare "2026-08-11" parses as midnight UTC and shows the day before west of Greenwich.
  const [y, m, d] = iso.split("-").map(Number);
  return new Date(Date.UTC(y, m - 1, d)).toLocaleDateString("en-GB", { day: "numeric", month: "long", year: "numeric", timeZone: "UTC" });
}

async function renderWhatsNew() {
  const box = $("#whats-new");
  let releases;
  try {
    const res = await fetch("changelog.json");
    releases = res.ok ? await res.json() : [];
  } catch {
    releases = [];
  }
  if (!releases.length) {
    box.replaceChildren(el("p", "about-text", "No release notes in this build."));
    return;
  }
  box.replaceChildren(
    ...releases.map((r) => {
      const section = el("section", "release");
      section.append(el("h4", null, `${r.version} · ${humanDate(r.date)}`));
      for (const b of r.blocks) {
        if (b.type === "list") {
          const ul = el("ul");
          for (const item of b.items) {
            const li = el("li");
            li.append(...segmentNodes(item));
            ul.append(li);
          }
          section.append(ul);
        } else {
          const p = el("p");
          p.append(...segmentNodes(b.text));
          section.append(p);
        }
      }
      return section;
    }),
  );
}

// ---------------------------------------------------------------------------
// Updates: published releases on GitHub (drafts never show up). Release builds only.
// ---------------------------------------------------------------------------

const LAST_VERSION_KEY = "chillsweep.lastVersion";

function showUpdateBar(text, { install = false } = {}) {
  $("#update-text").textContent = text;
  $("#update-install").hidden = !install;
  // The notes shipped in this build describe this build, so the link only fits "Updated to".
  $("#update-notes").hidden = install;
  $("#update-bar").hidden = false;
}

function newer(a, b) {
  const pa = a.split(".").map(Number), pb = b.split(".").map(Number);
  for (let i = 0; i < 3; i++) if (pa[i] !== pb[i]) return pa[i] > pb[i];
  return false;
}

/** Say once that an update landed, then look for the next one. */
async function checkUpdates() {
  try {
    const { version } = await invoke("app_info");
    let last = null;
    try {
      last = localStorage.getItem(LAST_VERSION_KEY);
      localStorage.setItem(LAST_VERSION_KEY, version);
    } catch {
      // Storage unavailable: skip the "updated" note, it's only a courtesy.
    }
    if (last && newer(version, last)) showUpdateBar(`Updated to ChillSweep ${version}.`);
  } catch {
    // No version to compare; carry on to the check.
  }

  let update = null;
  try {
    update = await invoke("check_update");
  } catch {
    return; // Offline, or GitHub unreachable. Nothing worth interrupting anyone for.
  }
  if (update) showUpdateBar(`ChillSweep ${update.version} is available.`, { install: true });
}

async function installUpdate() {
  const button = $("#update-install");
  button.disabled = true;
  button.querySelector(".label").textContent = "Downloading…";
  try {
    // On success the installer closes ChillSweep and reopens the new version; this doesn't return.
    await invoke("install_update");
  } catch (e) {
    // The pending update was used up, so the button goes; the next launch checks again.
    showUpdateBar(`The update didn't install: ${e}`);
    $("#update-notes").hidden = true;
    button.disabled = false;
    button.querySelector(".label").textContent = "Update and restart";
  }
}

async function openNotices() {
  const body = $("#notices-body");
  if (!body.textContent) {
    try {
      const res = await fetch("third-party-notices.txt");
      body.textContent = res.ok ? await res.text() : "The license file is missing from this build.";
    } catch (e) {
      body.textContent = `Couldn't load the license file: ${e}`;
    }
  }
  $("#notices").showModal();
}

window.addEventListener("DOMContentLoaded", () => {
  // Before anything renders: every label that names the bin or the file manager reads from OS,
  // and the defaults are deliberately vague ("the Recycle Bin or Trash"). Leaving this to
  // openAbout() meant the whole UI showed those placeholders until someone opened About.
  invoke("app_info").then(applyPlatform).catch(() => {});
  $("#about-open").addEventListener("click", () => openAbout());
  $("#update-notes").addEventListener("click", () => openAbout({ whatsNew: true }));
  $("#update-install").addEventListener("click", installUpdate);
  $("#update-dismiss").addEventListener("click", () => ($("#update-bar").hidden = true));
  checkUpdates();
  $("#notices-open").addEventListener("click", openNotices);
  for (const button of document.querySelectorAll("dialog [data-close]")) {
    button.addEventListener("click", () => button.closest("dialog").close());
  }
  // Clicking the backdrop closes the informational dialogs (not the clean-up confirmation).
  for (const dialog of [$("#about"), $("#notices")]) {
    dialog.addEventListener("click", (e) => {
      if (e.target !== dialog) return;
      const r = dialog.getBoundingClientRect();
      const inside = e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom;
      if (!inside) dialog.close();
    });
  }
  listen("scan-progress", (e) => {
    pendingScan = e.payload;
    schedule();
  });
  listen("clean-progress", (e) => onCleanProgress(e.payload));
  $("#scan").addEventListener("click", () => scan());
  $("#clean").addEventListener("click", confirmClean);
  $("#confirm-cancel").addEventListener("click", () => $("#confirm").close());
  $("#confirm-go").addEventListener("click", runClean);
  $("#undo").addEventListener("click", undoLast);
  $("#clear-selection").addEventListener("click", () => {
    selected.clear();
    updateSelectionBar();
  });
  updateSelectionBar();
});
