const { invoke } = window.__TAURI__.core;
const { revealItemInDir } = window.__TAURI__.opener;

const TIERS = {
  safe: { label: "Safe to clear", blurb: "Caches and junk that rebuild themselves." },
  leftover: { label: "Leftovers", blurb: "Left behind by apps and games that are no longer installed." },
  your_call: { label: "Your call", blurb: "Removable, but only you know if you still want these." },
  unknown: { label: "Unknown", blurb: "Couldn't reach a verdict, so these can't be removed. Here are the facts." },
};

const CATEGORY_LABELS = {
  cache: "Cache",
  leftover: "Leftover",
  temp: "Temp",
  stray: "Stray",
  dev: "Dev",
  games: "Games",
  downloads: "Downloads",
};

const $ = (sel) => document.querySelector(sel);

/** Findings from the current report, by id. */
let findings = new Map();
/** Ids of findings the user ticked. */
const selected = new Set();

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
// Report rendering
// ---------------------------------------------------------------------------

function renderSummary(report) {
  const summary = $("#summary");
  summary.replaceChildren();
  for (const total of report.totals) {
    const tier = TIERS[total.tier];
    const card = el("a", `card tier-${total.tier}`);
    card.href = `#tier-${total.tier}`;
    card.append(el("span", "card-label", tier.label), el("span", "card-size", formatSize(total.bytes)), el("span", "card-count", plural(total.count, "item")));
    if (total.count === 0) card.classList.add("empty");
    summary.append(card);
  }
  summary.hidden = false;
}

function renderFinding(f) {
  const node = $("#finding-template").content.firstElementChild.cloneNode(true);
  node.dataset.id = f.id;
  node.querySelector(".title").textContent = f.title;
  node.querySelector(".size").textContent = formatSize(f.bytes);

  const pick = node.querySelector(".pick");
  pick.setAttribute("aria-label", `Select ${f.title}`);
  if (f.tier === "unknown") {
    pick.disabled = true;
    pick.title = "Items without a verdict can't be removed.";
  }
  pick.addEventListener("change", () => {
    if (pick.checked) selected.add(f.id);
    else selected.delete(f.id);
    updateSelectionBar();
  });

  const chips = node.querySelector(".chips");
  chips.append(el("span", "chip", CATEGORY_LABELS[f.category] ?? f.category));
  if (f.confidence !== "high") chips.append(el("span", `chip conf-${f.confidence}`, `${f.confidence} confidence`));

  const what = node.querySelector(".what");
  if (f.what) what.textContent = f.what;
  else what.remove();

  const evidence = node.querySelector(".evidence");
  for (const e of f.evidence) evidence.append(el("li", null, e));

  const ifDeleted = node.querySelector(".if-deleted");
  if (f.if_deleted) ifDeleted.append(el("strong", null, "If you remove it: "), document.createTextNode(f.if_deleted));
  else ifDeleted.remove();

  const items = node.querySelector(".items");
  for (const it of f.items) {
    const li = el("li", "item");
    const path = el("span", "path", it.path);
    path.title = it.path;
    const meta = el("span", "item-size", it.is_dir ? `${formatSize(it.bytes)} · ${it.files.toLocaleString()} files` : formatSize(it.bytes));
    const reveal = el("button", "link", "Show in Explorer");
    reveal.type = "button";
    reveal.addEventListener("click", () => revealItemInDir(it.path).catch((e) => setStatus(`Couldn't open Explorer: ${e}`)));
    li.append(path, meta, reveal);
    items.append(li);
  }
  return node;
}

function renderResults(report) {
  const results = $("#results");
  results.replaceChildren();
  for (const total of report.totals) {
    if (total.count === 0) continue;
    const tier = TIERS[total.tier];
    const tierFindings = report.findings.filter((f) => f.tier === total.tier);
    const section = el("section", `tier tier-${total.tier}`);
    section.id = `tier-${total.tier}`;

    const head = el("div", "tier-head");
    head.append(el("h2", null, tier.label), el("span", "tier-total", formatSize(total.bytes)));
    if (total.tier !== "unknown") {
      const all = el("button", "link select-all", "Select all");
      all.type = "button";
      all.addEventListener("click", () => {
        const allSelected = tierFindings.every((f) => selected.has(f.id));
        for (const f of tierFindings) allSelected ? selected.delete(f.id) : selected.add(f.id);
        syncCheckboxes();
        updateSelectionBar();
      });
      head.append(all);
    }
    section.append(head, el("p", "tier-blurb", tier.blurb));

    const list = el("div", "findings");
    for (const f of tierFindings) list.append(renderFinding(f));
    section.append(list);
    results.append(section);
  }
}

function renderFooter(report) {
  const inv = report.inventory;
  const footer = $("#footer");
  footer.replaceChildren(
    el(
      "p",
      null,
      `Checked against ${inv.installed_programs} installed programs, ${inv.shortcuts} shortcuts, ${inv.running_processes} running programs, ` +
        `${inv.program_files_seen} Program Files folders and ${inv.steam_games} Steam games in ${(report.duration_ms / 1000).toFixed(1)}s.`,
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
  $("#selection-text").textContent = `${plural(chosen.length, "item")} selected · ${formatSize(bytes)}`;
  $("#selection-bar").hidden = chosen.length === 0;
  document.body.classList.toggle("has-selection", chosen.length > 0);
  syncCheckboxes();
}

// ---------------------------------------------------------------------------
// Scan, clean, undo
// ---------------------------------------------------------------------------

async function scan({ keepOutcome = false } = {}) {
  const button = $("#scan");
  button.disabled = true;
  button.textContent = "Scanning…";
  if (!keepOutcome) $("#outcome").hidden = true;
  setStatus("Scanning your folders. This only reads; nothing is changed.");
  try {
    const report = await invoke("scan");
    findings = new Map(report.findings.map((f) => [f.id, f]));
    selected.clear();
    $("#intro").hidden = true;
    renderSummary(report);
    renderResults(report);
    renderFooter(report);
    updateSelectionBar();
    const reclaimable = report.totals.filter((t) => t.tier !== "unknown").reduce((sum, t) => sum + t.bytes, 0);
    setStatus(`Found ${formatSize(reclaimable)} you could reclaim. Open any item to see why it was flagged, then tick what you want to remove.`);
  } catch (e) {
    setStatus(`Scan failed: ${e}`);
  } finally {
    button.disabled = false;
    button.textContent = "Scan again";
  }
}

function confirmClean() {
  const permanent = $("#permanent").checked;
  const chosen = selectedFindings();
  const groups = [
    { label: "Deleted permanently", note: "Frees space now. Can't be undone.", items: chosen.filter((f) => permanent && f.tier === "safe") },
    { label: "Moved to the Recycle Bin", note: "You can undo this, or restore items from the Recycle Bin later.", items: chosen.filter((f) => !(permanent && f.tier === "safe")) },
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

async function runClean() {
  $("#confirm").close();
  const ids = [...selected];
  const permanentSafe = $("#permanent").checked;
  $("#clean").disabled = true;
  setStatus(`Cleaning up ${plural(ids.length, "item")}…`);
  try {
    const result = await invoke("clean", { findingIds: ids, permanentSafe });
    showOutcome(result);
  } catch (e) {
    setStatus(`Clean-up failed: ${e}`);
    $("#clean").disabled = false;
    return;
  }
  $("#clean").disabled = false;
  await scan({ keepOutcome: true });
}

function showOutcome(result) {
  const parts = [];
  if (result.freed_bytes > 0) parts.push(`Freed ${formatSize(result.freed_bytes)}.`);
  if (result.recycled_bytes > 0) parts.push(`Moved ${formatSize(result.recycled_bytes)} to the Recycle Bin (empty it to free the space).`);
  if (parts.length === 0) parts.push("Nothing was removed.");
  $("#outcome-text").textContent = parts.join(" ");

  const problems = $("#outcome-problems");
  problems.replaceChildren();
  for (const o of result.outcomes.filter((o) => o.error)) {
    const li = el("li");
    li.append(el("strong", null, o.title || o.finding_id), document.createTextNode(` · ${o.path}: ${o.error}`));
    problems.append(li);
  }

  const recycled = result.outcomes.filter((o) => o.method === "recycle_bin" && !o.error).length;
  const undo = $("#undo");
  undo.hidden = recycled === 0;
  undo.disabled = false;
  undo.textContent = `Undo (put back ${plural(recycled, "item")})`;
  $("#outcome").hidden = false;
}

async function undoLast() {
  const undo = $("#undo");
  undo.disabled = true;
  try {
    const result = await invoke("undo_last");
    const problems = $("#outcome-problems");
    problems.replaceChildren(...result.failed.map((f) => el("li", null, `${f.path}: ${f.error}`)));
    $("#outcome-text").textContent = `Put back ${plural(result.restored.length, "item")} from the Recycle Bin.`;
    undo.hidden = true;
  } catch (e) {
    setStatus(`Undo failed: ${e}`);
    undo.disabled = false;
    return;
  }
  await scan({ keepOutcome: true });
}

window.addEventListener("DOMContentLoaded", () => {
  $("#scan").addEventListener("click", () => scan());
  $("#clean").addEventListener("click", confirmClean);
  $("#confirm-cancel").addEventListener("click", () => $("#confirm").close());
  $("#confirm-go").addEventListener("click", runClean);
  $("#undo").addEventListener("click", undoLast);
  $("#clear-selection").addEventListener("click", () => {
    selected.clear();
    updateSelectionBar();
  });
});
