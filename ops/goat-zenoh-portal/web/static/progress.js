import { readResponse, request, showRequestError } from "/app.js";
// /progress.html — vendor onboarding swimlane.
//
// Design: docs/design/portal-vendor-progress.md.
// Backend: GET /api/progress, operator-only.

import { api, hydrateAuth, hasAuth, lexicon } from "/app.js";

// Display lexicon (docs/design/portal-lexicon.md). Module-level; set by init
// before any render. Legacy fallback seed.
let LEX = { org: "vendor", orgs: "vendors", org_title: "Vendor", orgs_title: "Vendors" };

const tbody = document.getElementById("progress-rows");
const status = document.getElementById("progress-status");
const includeBox = document.getElementById("include-synthetic");

// Block 94 cross-surface deep-link sibling base URLs. Populated by
// init() from /api/siblings (best-effort). Empty fields suppress the
// per-row diagnose/tail anchors — URL-only contract, no runtime
// coupling. See docs/design/zenoh-surface-deep-links.md.
let siblings = { panoscope: "", stethoscope: "", flashlight: "" };

function appendSiblingAnchors(cell, vendorPrefix) {
  const q = `vendor=${encodeURIComponent(vendorPrefix)}`;
  if (siblings.stethoscope) {
    const a = document.createElement("a");
    a.className = "sibling-link";
    a.href = `${siblings.stethoscope}/?${q}`;
    a.target = "_blank";
    a.rel = "noopener noreferrer";
    a.title = `diagnose this ${LEX.org} in stethoscope`;
    a.textContent = "diagnose ↗";
    a.style.marginLeft = "0.5em";
    cell.appendChild(a);
  }
  if (siblings.flashlight) {
    const a = document.createElement("a");
    a.className = "sibling-link";
    a.href = `${siblings.flashlight}/?${q}`;
    a.target = "_blank";
    a.rel = "noopener noreferrer";
    a.title = `tail this ${LEX.org} in flashlight`;
    a.textContent = "tail ↗";
    a.style.marginLeft = "0.5em";
    cell.appendChild(a);
  }
  if (siblings.panoscope) {
    const a = document.createElement("a");
    a.className = "sibling-link";
    a.href = `${siblings.panoscope}/?${q}`;
    a.target = "_blank";
    a.rel = "noopener noreferrer";
    a.title = `see this ${LEX.org} in panoscope`;
    a.textContent = "panoscope ↗";
    a.style.marginLeft = "0.5em";
    cell.appendChild(a);
  }
}

function fmtTS(unix) {
  if (!unix) return null;
  try {
    const d = new Date(unix * 1000);
    if (isNaN(d.getTime())) return null;
    const y = d.getFullYear();
    const m = String(d.getMonth() + 1).padStart(2, "0");
    const day = String(d.getDate()).padStart(2, "0");
    const hh = String(d.getHours()).padStart(2, "0");
    const mm = String(d.getMinutes()).padStart(2, "0");
    return `${y}-${m}-${day} ${hh}:${mm}`;
  } catch (e) { return null; }
}

function cellTS(unix) {
  const td = document.createElement("td");
  td.className = "level-cell";
  const s = fmtTS(unix);
  if (s) {
    td.textContent = s;
    td.classList.add("level-reached");
  } else {
    td.textContent = "—";
    td.classList.add("level-unreached");
  }
  return td;
}

// Stage vocabulary, server-authoritative (GET /api/progress `stage`). The
// order here is display-only — it decides which stages read as "still early"
// for the summary line; the server decides which stage a row IS.
//
// There is deliberately no stage meaning "did not download" or "never
// published". A blank cell is absence of evidence, and rows predating a signal
// carry that ambiguity permanently — naming a negative would turn a gap in
// instrumentation into a claim about a person.
const STAGE_ORDER = [
  "registered",
  "bundle-minted",
  "bundle-downloaded",
  "mesh-enrolled",
  "topics-registered",
  "hello-world-published",
  "publishing",
];

function stageCell(stage) {
  const td = document.createElement("td");
  const pill = document.createElement("span");
  pill.className = "pill";
  // Only the far end of the ladder is "good"; everything before it is a place
  // an onboarding can sit, not a failure — so no alarm colouring in between.
  if (stage === "publishing") pill.classList.add("pill-ok");
  else if (stage === "registered") pill.classList.add("pill-warn");
  pill.textContent = stage || "unknown";
  td.appendChild(pill);
  return td;
}

// Bundle cell — minted vs actually fetched. These are different facts and the
// column exists because conflating them hid a stalled cohort: three partners
// read as "L1, no peers" when they had in fact collected and enrolled.
function bundleCell(row) {
  const td = document.createElement("td");
  // The two timestamps are the point of this cell; let them use the width they
  // need rather than wrapping a date across three lines.
  td.style.whiteSpace = "nowrap";
  const minted = fmtTS(row.l1_at);
  if (!minted) {
    td.textContent = "—";
    td.classList.add("level-unreached");
    return td;
  }
  const top = document.createElement("div");
  top.textContent = `minted ${minted}`;
  td.appendChild(top);

  const got = fmtTS(row.bundle_downloaded_at);
  const sub = document.createElement("div");
  sub.className = "hint";
  if (got) {
    const n = row.bundle_download_count || 1;
    sub.textContent = `fetched ${got}${n > 1 ? ` (${n}×)` : ""}`;
  } else {
    // Not "never fetched" — downloads have only been recorded since
    // so an older row is genuinely unknown here.
    sub.textContent = "no fetch recorded";
    sub.title =
      "No download has been recorded for this bundle. Downloads have only been " +
      "recorded since 2026-08-12, so for an older row this means unknown, not never.";
  }
  td.appendChild(sub);
  return td;
}

// Monotonic display rule — once a higher level is reached, lower
// levels are considered reached even if their timestamp is NULL. We
// keep the actual timestamps in their cells so the operator can still
// see "they skipped hello and went straight to L4 at HH:MM"; the
// rendered cell just gets the same "reached" pill background.
//
// Indices are the four level cells; they sit after the org name, stage and
// bundle columns.
function applyMonotonic(row, l1, l2, l3, l4) {
  const cells = [row.children[3], row.children[4], row.children[5], row.children[6]];
  const reached = [!!l1, !!l2, !!l3, !!l4];
  let max = -1;
  for (let i = 0; i < 4; i++) if (reached[i]) max = i;
  for (let i = 0; i < 4; i++) {
    if (i < max && !reached[i]) {
      cells[i].classList.remove("level-unreached");
      cells[i].classList.add("level-implied");
      if (cells[i].textContent === "—") cells[i].textContent = "(implied)";
    }
  }
}

function render(rows) {
  tbody.innerHTML = "";
  if (!rows.length) {
    const tr = document.createElement("tr");
    const td = document.createElement("td");
    td.colSpan = 10;
    td.className = "hint";
    td.textContent = `No ${LEX.orgs} yet. Once you mint a bundle here, that ${LEX.org} lands on L1.`;
    tr.appendChild(td);
    tbody.appendChild(tr);
    status.textContent = `0 ${LEX.orgs}`;
    return;
  }
  for (const r of rows) {
    const tr = document.createElement("tr");

    const vCell = document.createElement("td");
    const link = document.createElement("a");
    link.href = `/vendor.html?vendor=${encodeURIComponent(r.prefix)}`;
    link.textContent = r.display_name && r.display_name !== r.prefix
      ? `${r.display_name} (${r.prefix})`
      : r.prefix;
    vCell.appendChild(link);
    if (r.synthetic) {
      const pill = document.createElement("span");
      pill.className = "pill pill-warn";
      pill.textContent = "SYNTHETIC";
      pill.style.marginLeft = "0.5em";
      vCell.appendChild(pill);
    }
    // Block 94 cross-surface deep-link buttons (no-op when no siblings
    // configured at this deployment).
    appendSiblingAnchors(vCell, r.prefix);
    tr.appendChild(vCell);

    tr.appendChild(stageCell(r.stage));
    tr.appendChild(bundleCell(r));

    tr.appendChild(cellTS(r.l1_at));
    tr.appendChild(cellTS(r.l2_at));
    tr.appendChild(cellTS(r.l3_at));
    tr.appendChild(cellTS(r.l4_at));

    const pCell = document.createElement("td");
    pCell.textContent = r.peer_count || 0;
    tr.appendChild(pCell);

    const lpCell = document.createElement("td");
    lpCell.textContent = fmtTS(r.last_publish_at) || "—";
    tr.appendChild(lpCell);

    // Activity summary — what the vendor has actually published, rolled
    // up from the audit stream. "—" until the tailer sees a publish.
    const aCell = document.createElement("td");
    const pub = r.publish_count || 0;
    const observed = r.distinct_topics || 0;
    const declared = r.registered_topics || 0;
    if (pub > 0 || declared > 0) {
      const summary = document.createElement("div");
      summary.textContent =
        `${declared} declared · ${observed} observed`;
      aCell.appendChild(summary);

      const pubLine = document.createElement("div");
      pubLine.className = "hint";
      pubLine.textContent = `${pub} publish${pub === 1 ? "" : "es"}`;
      aCell.appendChild(pubLine);

      // The two gaps worth naming in place, because each has a different
      // remedy and neither is visible from a single merged topic count.
      if (declared > 0 && observed === 0) {
        const w = document.createElement("div");
        w.className = "hint";
        w.textContent = "declared, never published";
        aCell.appendChild(w);
      } else if (observed > 0 && declared === 0) {
        const w = document.createElement("div");
        w.className = "hint";
        w.textContent = "publishing undeclared topics — decode surfaces cannot render these";
        aCell.appendChild(w);
      }
      if (r.last_topic) {
        const lt = document.createElement("div");
        lt.className = "hint";
        lt.textContent = `latest: ${r.last_topic}`;
        aCell.appendChild(lt);
      }
    } else {
      aCell.textContent = "—";
      aCell.classList.add("level-unreached");
    }
    tr.appendChild(aCell);

    applyMonotonic(tr, r.l1_at, r.l2_at, r.l3_at, r.l4_at);
    tbody.appendChild(tr);
  }
  // Lead the summary with the number that prompts action: how many are sitting
  // short of publishing. The L-counts stay behind it as the raw tally.
  const publishing = rows.filter(r => r.stage === "publishing").length;
  const shortOf = rows.length - publishing;
  const byStage = new Map();
  for (const r of rows) {
    const s = r.stage || "unknown";
    byStage.set(s, (byStage.get(s) || 0) + 1);
  }
  const stalls = STAGE_ORDER
    .filter(s => s !== "publishing" && byStage.has(s))
    .map(s => `${s}: ${byStage.get(s)}`)
    .join(" · ");

  const reached4 = rows.filter(r => r.max_level >= 4).length;
  const reached3 = rows.filter(r => r.max_level >= 3).length;
  const reached2 = rows.filter(r => r.max_level >= 2).length;
  const reached1 = rows.filter(r => r.max_level >= 1).length;
  status.textContent =
    `${rows.length} ${rows.length === 1 ? LEX.org : LEX.orgs}` +
    ` · ${publishing} publishing, ${shortOf} short of it` +
    (stalls ? ` (${stalls})` : "") +
    ` · L1: ${reached1} · L2: ${reached2} · L3: ${reached3} · L4: ${reached4}`;
}

async function load() {
  status.textContent = "loading…";
  try {
    const path = includeBox.checked
      ? "/api/progress?include=synthetic"
      : "/api/progress";
    const data = await api(path);
    render(Array.isArray(data) ? data : []);
  } catch (e) {
    tbody.innerHTML = "";
    status.textContent = "error: " + e.message;
  }
}

async function init() {
  await hydrateAuth();
  LEX = await lexicon();
  const section = document.getElementById("progress-section");
  if (!hasAuth()) {
    section.classList.add("hidden");
    return;
  }
  section.classList.remove("hidden");

  // Block 94 cross-surface deep-link contract — pull sibling URLs
  // best-effort before the first render so the per-row anchors land
  // immediately. Failure leaves `siblings` at its empty default and
  // appendSiblingAnchors becomes a no-op.
  try {
    const resp = await request("/api/siblings", { headers: { Accept: "application/json" } });
    const got = await readResponse(resp);
      siblings = {
        panoscope: got.panoscope || "",
        stethoscope: got.stethoscope || "",
        flashlight: got.flashlight || "",
      };
  } catch (e) { showRequestError(e); }

  includeBox.addEventListener("change", load);
  await load();
  // Refresh every 60s so the operator sees the swimlane move in
  // near-real-time without hammering F5.
  setInterval(load, 60000);
}

init();
