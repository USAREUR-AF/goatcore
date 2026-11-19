// SPDX-License-Identifier: Apache-2.0
// goat-zenoh-portal — Pod Rules Waiting: the org lead's ratification queue for
// local rules a cut-off pod made (ADR 1251; docs/design/pod-carriage-and-provenance.md
// §4 and §7.5). It lists GET /api/carriage/local-rules?state=pending and calls
// POST /api/carriage/local-rules/{id}/ratify or /lapse. The portal decides who
// may see and decide each rule; this page holds no rule of its own.

import { api, escapeHTML, hydrateAuth, showRequestError } from "/app.js";

const $ = (id) => document.getElementById(id);

const ACTION_WORDS = {
  allow: "Let in",
  extend: "Extend",
  revoke: "Shut out",
  reactivate: "Bring back",
};

// The portal sends times as Unix seconds; an RFC 3339 string is shown as given.
function when(t) {
  if (t === null || t === undefined || t === "" || t === 0) return "no end date";
  const d = typeof t === "number" ? new Date(t * 1000) : new Date(t);
  if (Number.isNaN(d.getTime())) return String(t);
  return d.toISOString().replace("T", " ").replace(/:\d\d(\.\d+)?Z$/, " UTC");
}

function rowsOf(resp) {
  return resp.local_rules || [];
}

async function load() {
  const el = $("pending");
  const status = $("pending-status");
  let rows;
  try {
    rows = rowsOf(await api("/api/carriage/local-rules?state=pending"));
  } catch (e) {
    el.replaceChildren();
    showRequestError(e, status, load);
    return;
  }
  status.classList.remove("warn");
  status.textContent = rows.length
    ? `${rows.length} change${rows.length === 1 ? "" : "s"} waiting.`
    : "Nothing is waiting. Every rule made on a pod has been decided.";
  if (!rows.length) {
    el.replaceChildren();
    return;
  }
  el.innerHTML = `<table><thead><tr>
      <th>Pod</th><th>Change</th><th>Source</th><th>Until</th><th>Reason</th><th>Made by</th><th>Made</th><th></th>
    </tr></thead><tbody>${
    rows.map((r) => `<tr data-id="${escapeHTML(r.id)}">
      <td><code>${escapeHTML(r.pod_slot)}</code></td>
      <td>${escapeHTML(ACTION_WORDS[r.action] || r.action)}${r.pod_rule_id ? ` <span class="hint">rule ${escapeHTML(r.pod_rule_id)}</span>` : ""}</td>
      <td><code>${escapeHTML(r.source_slot)}</code></td>
      <td>${escapeHTML(when(r.until))}</td>
      <td>${escapeHTML(r.reason || "")}</td>
      <td>${escapeHTML(r.authority_identity || r.authority_cn || "")}</td>
      <td>${escapeHTML(when(r.entry_ts))}</td>
      <td>
        <button type="button" data-ratify="${escapeHTML(r.id)}" title="goat carriage ratify ${escapeHTML(r.id)}">Ratify</button>
        <button type="button" data-lapse="${escapeHTML(r.id)}" title="goat carriage lapse ${escapeHTML(r.id)}">Let lapse</button>
      </td>
    </tr>`).join("")
  }</tbody></table>`;
  el.querySelectorAll("[data-ratify]").forEach((b) =>
    b.addEventListener("click", () => decide(rows.find((r) => r.id === b.getAttribute("data-ratify")), "ratify")));
  el.querySelectorAll("[data-lapse]").forEach((b) =>
    b.addEventListener("click", () => decide(rows.find((r) => r.id === b.getAttribute("data-lapse")), "lapse")));
}

// The result is said before it runs, with the command it equals.
function preview(r, verb) {
  const change = ACTION_WORDS[r.action] || r.action;
  const result = verb === "ratify"
    ? `Keep "${change}" for source ${r.source_slot} on pod ${r.pod_slot} as a goat rule, until ${when(r.until)}.`
    : `End "${change}" for source ${r.source_slot} on pod ${r.pod_slot}. The pod drops it at its next rule update.`;
  return `${result}\n\nSame as: goat carriage ${verb} ${r.id}`;
}

async function decide(r, verb) {
  if (!r || !window.confirm(preview(r, verb))) return;
  const status = $("pending-status");
  try {
    await api(`/api/carriage/local-rules/${encodeURIComponent(r.id)}/${verb}`, { method: "POST", body: "{}" });
    await load();
    status.textContent = `${verb === "ratify" ? "Ratified" : "Let lapse"}: ${r.id}. ` + status.textContent;
  } catch (e) {
    showRequestError(e, status, () => decide(r, verb));
  }
}

hydrateAuth().then((auth) => { if (auth) load(); });
