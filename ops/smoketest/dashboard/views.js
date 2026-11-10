// SPDX-License-Identifier: Apache-2.0
// views.js — render functions. Each takes data and returns DOM nodes; text is
// set with textContent only, so nothing from the API is parsed as HTML.

export function h(tag, attrs, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v == null || v === false) continue;
    if (k === "class") el.className = v;
    else if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k === "style") Object.assign(el.style, v);
    else el.setAttribute(k, v === true ? "" : v);
  }
  for (const c of children.flat()) {
    if (c == null || c === false) continue;
    el.append(c instanceof Node ? c : String(c));
  }
  return el;
}

const STATUSES = new Set(["PASS", "FAIL", "WARN", "SKIP"]);
const statusClass = (s) => "s-" + (STATUSES.has(s) ? s : "SKIP");
const ms = (v) => `${v.toFixed(1)}ms`;
const emptyCard = (text) => h("div", { class: "card empty" }, text);
const displayName = (name) => (name.startsWith("svc-") ? name.slice(4) : name);

// --- summary strip --------------------------------------------------------
export function summary(run) {
  const pill = (label, value, cls) =>
    h("div", { class: "pill" },
      h("div", { class: `pill-value ${cls}` }, value),
      h("div", { class: "pill-label" }, label));
  return [
    pill("Pass", run.pass, "c-pass"),
    pill("Fail", run.fail, "c-fail"),
    pill("Warn", run.warn, "c-warn"),
    pill("Total", (run.checks || []).length, "dim"),
  ];
}

// --- run history ----------------------------------------------------------
function runStatus(run) {
  if (run.fail > 0) return "FAIL";
  if (run.warn > 0) return "WARN";
  return "PASS";
}

function shortTime(iso) {
  return new Date(iso).toLocaleString(undefined, {
    month: "short", day: "numeric", hour: "2-digit", minute: "2-digit",
  });
}

export function history(runs, selectedId, onSelect) {
  if (!runs.length) return h("div", { class: "empty" }, "No run history.");
  return h("div", { class: "history" },
    runs.map((run) =>
      h("button", {
        type: "button",
        class: `history-run ${statusClass(runStatus(run))}${run.run_id === selectedId ? " selected" : ""}`,
        title: `${shortTime(run.timestamp)} — ${run.pass}P ${run.fail}F ${run.warn}W (${run.duration_s.toFixed(1)}s)`,
        "aria-label": `Run ${run.run_id}`,
        onclick: () => onSelect(run),
      })));
}

// --- results matrix -------------------------------------------------------
const CATEGORY_ORDER = ["dns", "tls", "relay", "stun", "mesh", "service", "latency", "load"];
const CATEGORY_LABEL = {
  dns: "DNS", tls: "TLS", relay: "Relay", stun: "STUN",
  mesh: "Mesh", service: "Service", latency: "Latency", load: "Load",
};

export function results(checks) {
  if (!checks.length) return emptyCard("No results yet. Run a smoke test to see results.");
  const grouped = new Map();
  for (const c of checks) {
    if (!grouped.has(c.category)) grouped.set(c.category, []);
    grouped.get(c.category).push(c);
  }
  const rows = [];
  for (const cat of CATEGORY_ORDER.filter((c) => grouped.has(c))) {
    rows.push(h("tr", { class: "cat-row" }, h("td", { colspan: 4 }, CATEGORY_LABEL[cat])));
    for (const r of grouped.get(cat)) {
      rows.push(h("tr", { class: `result-row ${statusClass(r.status)}` },
        h("td", {}, displayName(r.name)),
        h("td", {}, h("span", { class: `badge ${statusClass(r.status)}` }, r.status)),
        h("td", { class: "detail", title: r.detail }, r.detail),
        h("td", { class: "num" }, r.latency_ms != null ? ms(r.latency_ms) : "—")));
    }
  }
  return h("div", { class: "card" },
    h("table", { class: "table" },
      h("thead", {}, h("tr", {},
        h("th", {}, "Check"),
        h("th", { style: { width: "70px" } }, "Status"),
        h("th", {}, "Detail"),
        h("th", { style: { width: "80px", textAlign: "right" } }, "Latency"))),
      h("tbody", {}, rows)));
}

// --- relay latency --------------------------------------------------------
export function latency(checks) {
  const rows = checks.filter((c) => c.category === "latency" && c.latency_ms != null);
  if (!rows.length) return emptyCard("No latency data available.");
  const values = rows.map((c) => c.latency_ms);
  const max = Math.max(...values, 1);
  const sorted = [...values].sort((a, b) => a - b);
  const avg = values.reduce((s, v) => s + v, 0) / values.length;
  const p50 = sorted[Math.floor(sorted.length * 0.5)];
  const p95 = sorted[Math.floor(sorted.length * 0.95)];
  const band = (v) => (v < 50 ? "bg-pass" : v < 150 ? "bg-warn" : "bg-fail");
  const stat = (label, v) => h("div", {}, h("span", { class: "dim" }, `${label}: `), h("b", {}, ms(v)));
  return h("div", { class: "card" },
    h("div", { class: "latency-stats" }, stat("Avg", avg), stat("P50", p50), stat("P95", p95)),
    h("div", { class: "latency-rows" },
      rows.map((c) =>
        h("div", { class: "latency-row" },
          h("div", { class: "latency-name", title: c.name }, c.name),
          h("div", { class: "latency-track" },
            h("div", {
              class: `latency-bar ${band(c.latency_ms)}`,
              style: { width: `${Math.max((c.latency_ms / max) * 100, 2)}%` },
            })),
          h("div", { class: "latency-ms" }, ms(c.latency_ms))))));
}

// --- service health -------------------------------------------------------
export function probeMap(probes) {
  // Probe names carry an "svc-" prefix; the catalogue names do not.
  const map = new Map();
  for (const p of probes) map.set(displayName(p.name), p);
  return map;
}

export function services(catalogue, probes, selectedName, onSelect) {
  if (!catalogue.length) return emptyCard("No services in catalogue.");
  return h("div", { class: "grid" },
    catalogue.map((svc) => {
      const probe = probes.get(svc.name);
      const st = statusClass(probe ? probe.status : "SKIP");
      return h("button", {
        type: "button",
        class: `card svc ${st}${selectedName === svc.name ? " selected" : ""}`,
        onclick: () => onSelect(svc),
      },
        h("div", { class: "svc-head" },
          h("span", { class: "svc-name" }, h("span", { class: "dot" }), svc.name),
          h("span", { class: "badge plain" }, svc.category)),
        h("div", { class: "svc-desc" }, svc.description),
        probe && h("div", { class: "svc-probe" },
          probe.latency_ms != null && h("span", { class: "ms" }, ms(probe.latency_ms)),
          h("span", {}, probe.detail)));
    }));
}

const PHASES = ["dns_ms", "tcp_ms", "tls_ms", "ttfb_ms"];
const PHASE_LABEL = { dns_ms: "DNS", tcp_ms: "TCP", tls_ms: "TLS", ttfb_ms: "TTFB" };

function phaseBreakdown(meta) {
  const present = PHASES.filter((p) => meta[p] != null);
  if (!present.length) return null;
  const total = meta.total_ms || present.reduce((s, p) => s + (meta[p] || 0), 0);
  const maxBar = 200; // px
  return h("div", { class: "phases" },
    h("div", { class: "phases-title" }, "Latency Breakdown"),
    present.map((p) => {
      const v = meta[p];
      const pct = total > 0 ? (v / total) * 100 : 0;
      const width = total > 0 ? Math.max(2, (v / total) * maxBar) : 2;
      return h("div", { class: `phase phase-${p}` },
        h("span", { class: "phase-label" }, PHASE_LABEL[p]),
        h("div", { class: "phase-track" }, h("div", { class: "phase-bar", style: { width: `${width}px` } })),
        h("span", { class: "phase-ms" }, ms(v)),
        h("span", { class: "phase-pct" }, `${pct.toFixed(0)}%`));
    }));
}

export function serviceDetail(svc, probe, onClose) {
  const st = statusClass(probe ? probe.status : "SKIP");
  const field = (label, ...value) => h("div", {}, h("span", { class: "label" }, `${label}:`), " ", ...value);
  return h("div", {
    class: `card dialog ${st}`,
    role: "dialog",
    "aria-modal": "true",
    "aria-label": svc.name,
    onclick: (e) => e.stopPropagation(),
  },
    h("button", { type: "button", class: "dialog-close", "aria-label": "Close", onclick: onClose }, "x"),
    h("div", { class: "dialog-head" },
      h("span", { class: "dot" }),
      h("h3", {}, svc.name),
      h("span", { class: "badge plain" }, svc.category)),
    h("p", {}, svc.description),
    svc.url && h("p", { class: "dim" }, h("span", { class: "label" }, "URL:"), " ", h("span", { class: "mono" }, svc.url)),
    probe
      ? h("div", { class: "probe-box" },
          field("Status", h("span", { class: "status-text" }, probe.status)),
          probe.latency_ms != null && field("Total Latency", ms(probe.latency_ms)),
          probe.metadata && phaseBreakdown(probe.metadata),
          field("Detail", h("span", { class: "mono" }, probe.detail)),
          field("Checked", new Date(probe.timestamp).toLocaleString()))
      : h("p", { class: "dim" }, h("i", {}, "No probe results yet — run a smoke test to check this service.")));
}

// --- mesh peers -----------------------------------------------------------
function relativeTime(iso) {
  if (!iso) return "—";
  const d = new Date(iso);
  const diff = Date.now() - d.getTime();
  if (diff < 60_000) return "just now";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return d.toLocaleDateString();
}

const PEER_SORT = {
  name: (a, b) => a.name.localeCompare(b.name),
  ip: (a, b) => a.ip.localeCompare(b.ip),
  connected: (a, b) => (a.connected ? 1 : 0) - (b.connected ? 1 : 0),
  last_seen: (a, b) => new Date(a.last_seen).getTime() - new Date(b.last_seen).getTime(),
  os: (a, b) => a.os.localeCompare(b.os),
};

export function peers(list, sort, onSort) {
  const sorted = [...list].sort((a, b) => (sort.asc ? 1 : -1) * PEER_SORT[sort.key](a, b));
  const connected = list.filter((p) => p.connected).length;
  const th = (key, label) =>
    h("th", { class: "sortable", onclick: () => onSort(key) },
      label, sort.key === key ? (sort.asc ? " ▲" : " ▼") : "");
  return h("div", { class: "card" },
    h("div", { class: "peer-count" },
      h("b", { class: "c-pass" }, connected), " connected / ", h("b", {}, list.length), " total"),
    h("table", { class: "table" },
      h("thead", {}, h("tr", {},
        th("name", "Name"), th("ip", "IP"), th("connected", "Status"),
        th("last_seen", "Last Seen"), th("os", "OS"))),
      h("tbody", {},
        sorted.map((p) =>
          h("tr", { class: p.connected ? "" : "peer-offline" },
            h("td", { style: { fontWeight: 500 } }, p.name),
            h("td", { class: "mono", style: { fontSize: "12px" } }, p.ip),
            h("td", {}, h("span", { class: `badge ${p.connected ? "s-PASS" : "s-FAIL"}` },
              p.connected ? "Connected" : "Offline")),
            h("td", { class: "dim", style: { fontSize: "12px" } }, relativeTime(p.last_seen)),
            h("td", { class: "dim", style: { fontSize: "12px" } }, p.os))))),
    !list.length && h("div", { class: "empty", style: { padding: "20px", textAlign: "center" } }, "No peers found."));
}
