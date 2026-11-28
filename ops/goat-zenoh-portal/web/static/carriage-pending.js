// SPDX-License-Identifier: Apache-2.0
// goat-zenoh-portal — Pod Rules Waiting: the org lead's ratification queue for
// local rules a cut-off pod made (ADR 1251; docs/design/pod-carriage-and-provenance.md
// §4 and §7.5). It lists GET /api/carriage/local-rules?state=pending and calls
// POST /api/carriage/local-rules/{id}/ratify or /lapse. The portal decides who
// may see and decide each rule; this page holds no rule of its own.

import { api, ApiError, escapeHTML, hydrateAuth, readResponse, request, showRequestError } from "/app.js";

const $ = (id) => document.getElementById(id);

const ACTION_WORDS = {
  allow: "Let in",
  extend: "Extend",
  revoke: "Shut out",
  reactivate: "Bring back",
};

// Refusals the carriage routes return, in plain words (codes from the portal's
// carriage handlers). Anything else falls back to the portal's own message.
const REFUSALS = {
  local_rule_not_pending: "Someone already decided this change. The list now shows what is still waiting.",
  local_rule_expired: "This change has already ended on the pod, so there is nothing to keep. Let it lapse instead.",
  carriage_source_revoked: "goat has revoked this source. A revoke made in goat always wins, so this change has already been let lapse.",
  not_found: "This change is no longer there. The list now shows what is still waiting.",
  ledger_conflict: "The pod's record of this change does not match what goat holds. Ask a goat operator to check the pod's ledger before you decide.",
  carriage_not_authorized: "Only an org lead of the pod's org can decide this change.",
  config: "The portal is not set up for pod rules yet. Ask a goat operator.",
};

async function post(path) {
  const res = await request(path, {
    method: "POST",
    headers: { Accept: "application/json", "Content-Type": "application/json" },
    body: "{}",
  });
  if (!res.ok) {
    let body = null;
    try { body = await res.clone().json(); } catch (_) { body = null; }
    const code = body?.code || body?.error?.code;
    if (code && REFUSALS[code]) {
      const e = new ApiError(REFUSALS[code], res.status, "request");
      e.code = code;
      throw e;
    }
  }
  return readResponse(res);
}

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
    : "Nothing is waiting for you. You see the pods of the orgs you lead.";
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
      <td>${r.action === "revoke" ? "at once" : escapeHTML(when(r.until))}</td>
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

// The result is said before it runs, with the command it equals, in an in-page
// sheet. Resolves true only when the person presses the yes button.
function confirmSheet(r, verb) {
  const change = ACTION_WORDS[r.action] || r.action;
  const ratify = verb === "ratify";
  $("decide-title").textContent = ratify ? `Ratify ${r.id}?` : `Let ${r.id} lapse?`;
  const kept = r.action === "revoke"
    ? `Result: source ${r.source_slot} stays shut out of pod ${r.pod_slot}; goat keeps this change.`
    : `Result: "${change}" for source ${r.source_slot} on pod ${r.pod_slot} is kept as a goat rule, until ${when(r.until)}.`;
  $("decide-result").textContent = ratify
    ? kept
    : `Result: "${change}" for source ${r.source_slot} on pod ${r.pod_slot} ends. The pod drops it at its next rule update.`;
  $("decide-cli").textContent = `goat carriage ${verb} ${r.id}`;
  $("decide-yes").textContent = ratify ? "Yes, ratify" : "Yes, let lapse";
  const sheet = $("decide-sheet");
  return new Promise((resolve) => {
    const finish = (v) => {
      $("decide-yes").removeEventListener("click", yes);
      $("decide-no").removeEventListener("click", no);
      sheet.removeEventListener("cancel", cancel);
      if (sheet.open) sheet.close();
      resolve(v);
    };
    const yes = () => finish(true);
    const no = () => finish(false);
    const cancel = (ev) => { ev.preventDefault(); finish(false); };
    $("decide-yes").addEventListener("click", yes);
    $("decide-no").addEventListener("click", no);
    sheet.addEventListener("cancel", cancel);
    sheet.showModal();
    $("decide-no").focus();
  });
}

async function decide(r, verb) {
  if (!r || !(await confirmSheet(r, verb))) return;
  const status = $("pending-status");
  try {
    await post(`/api/carriage/local-rules/${encodeURIComponent(r.id)}/${verb}`);
    await load();
    status.textContent = `${verb === "ratify" ? "Ratified" : "Let lapse"}: ${r.id}. ` + status.textContent;
  } catch (e) {
    if (["local_rule_not_pending", "carriage_source_revoked", "not_found"].includes(e.code)) await load();
    showRequestError(e, status, () => decide(r, verb));
  }
}

hydrateAuth().then((auth) => { if (auth) load(); });
