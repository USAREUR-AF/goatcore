// SPDX-License-Identifier: Apache-2.0
// app.js — smoketest dashboard. Plain ES modules served as static files by
// `smoketest serve --dashboard-dir`; no build step.
import * as api from "./api.js";
import * as view from "./views.js";

const $ = (id) => document.getElementById(id);

const state = {
  latest: null,
  selected: null,
  history: [],
  peers: [],
  peerSort: { key: "name", asc: true },
  catalogue: [],
  probes: new Map(),
  servicesError: false,
  selectedService: null,
  mode: "quick",
  run: null, // last /api/run/status
};

function active() {
  return state.selected || state.latest;
}

// --- render ---------------------------------------------------------------
function renderHeader() {
  const l = state.latest;
  $("last-run").textContent = l
    ? ` — last run ${new Date(l.timestamp).toLocaleString()} (${l.duration_s.toFixed(1)}s, ${l.mode})`
    : "";
}

function renderRunner() {
  const running = !!(state.run && state.run.running);
  for (const b of document.querySelectorAll(".toggle-opt")) {
    const on = b.dataset.mode === state.mode;
    b.classList.toggle("active", on);
    b.setAttribute("aria-pressed", String(on));
    b.disabled = running;
  }
  const btn = $("run-btn");
  btn.disabled = running;
  btn.textContent = running ? "Running..." : "Run Smoke Test";
  const elapsed = $("run-elapsed");
  elapsed.hidden = !running;
  if (running) elapsed.textContent = `${state.run.elapsed_s.toFixed(1)}s elapsed`;
}

function renderRun() {
  const run = active();
  const summary = $("summary");
  summary.hidden = !run;
  summary.replaceChildren(...(run ? view.summary(run) : []));

  $("history").replaceChildren(
    view.history(state.history, run && run.run_id, (r) => {
      state.selected = r;
      renderRun();
    }));

  const viewing = $("viewing");
  const older = run && state.latest && run.run_id !== state.latest.run_id;
  viewing.hidden = !older;
  if (older) viewing.textContent = `(viewing ${run.run_id.substring(0, 8)})`;

  const checks = (run && run.checks) || [];
  $("results").replaceChildren(view.results(checks));
  $("latency").replaceChildren(view.latency(checks));
}

function renderServices() {
  const el = $("services");
  if (state.servicesError) {
    el.replaceChildren(view.h("div", { class: "card empty" }, "Unable to load service health data."));
    return;
  }
  el.replaceChildren(
    view.services(state.catalogue, state.probes, state.selectedService && state.selectedService.name, openService));
  const overlay = $("overlay");
  const svc = state.selectedService;
  overlay.hidden = !svc;
  overlay.replaceChildren(...(svc ? [view.serviceDetail(svc, state.probes.get(svc.name) || null, closeService)] : []));
}

function renderPeers() {
  $("peers").replaceChildren(
    view.peers(state.peers, state.peerSort, (key) => {
      const s = state.peerSort;
      state.peerSort = s.key === key ? { key, asc: !s.asc } : { key, asc: true };
      renderPeers();
    }));
}

function openService(svc) {
  state.selectedService = svc;
  renderServices();
}

function closeService() {
  state.selectedService = null;
  renderServices();
}

// --- data -----------------------------------------------------------------
async function loadData() {
  // Settled one by one: /api/peers answers 503 when no NetBird token is
  // configured, and that must not hide the run results.
  const [latest, history, peers] = await Promise.allSettled([
    api.getLatestResult(),
    api.getResults(30),
    api.getPeers(),
  ]);
  if (latest.status === "fulfilled") state.latest = latest.value;
  if (history.status === "fulfilled") state.history = history.value || [];
  if (peers.status === "fulfilled") state.peers = peers.value || [];
  if (!state.selected && state.latest) state.selected = state.latest;
  renderHeader();
  renderRun();
  renderPeers();
}

async function loadServices() {
  try {
    const [catalogue, svc] = await Promise.all([api.getCatalogue(), api.getServices()]);
    state.catalogue = catalogue || [];
    state.probes = view.probeMap((svc && svc.checks) || []);
  } catch {
    state.servicesError = true;
  }
  renderServices();
}

async function refreshPeers() {
  try {
    state.peers = (await api.getPeers()) || [];
    renderPeers();
  } catch {
    // keep the last list
  }
}

// --- run control ----------------------------------------------------------
let poll = null;

function startPolling() {
  if (poll) return;
  poll = setInterval(async () => {
    try {
      state.run = await api.getRunStatus();
      renderRunner();
      if (!state.run.running) {
        stopPolling();
        state.selected = null; // show the new latest run
        loadData();
      }
    } catch {
      stopPolling();
    }
  }, 1000);
}

function stopPolling() {
  clearInterval(poll);
  poll = null;
}

function showError(msg) {
  const el = $("run-error");
  el.hidden = !msg;
  el.textContent = msg || "";
}

async function handleRun() {
  showError(null);
  try {
    await api.triggerRun(state.mode);
    state.run = await api.getRunStatus();
    renderRunner();
    startPolling();
  } catch (e) {
    showError(e.status === 409 ? "A test run is already in progress" : e.message || "Failed to start run");
  }
}

// --- wire up --------------------------------------------------------------
for (const b of document.querySelectorAll(".toggle-opt")) {
  b.addEventListener("click", () => {
    state.mode = b.dataset.mode;
    renderRunner();
  });
}
$("run-btn").addEventListener("click", handleRun);
$("overlay").addEventListener("click", closeService);
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && state.selectedService) closeService();
});

api.getRunStatus()
  .then((s) => {
    state.run = s;
    renderRunner();
    if (s.running) startPolling();
  })
  .catch(() => {});

renderRunner();
loadData();
loadServices();
setInterval(refreshPeers, 30_000);
