// goat-zenoh-portal — shared frontend logic.
//
// Vanilla JS, ES modules. No framework, no bundler. Format decoders are
// loaded on-demand from /decoders.js. Lives behind the portal API key in
// an opaque HttpOnly session cookie; API keys stay in server memory.

import { formatUptime, compareDisplayNames } from "/display.js";

// Remove credentials persisted by older clients without reusing them.
try { localStorage.removeItem("portalApiKey"); } catch (_) {}
let authenticated = false;
export function hasAuth() { return authenticated; }
export async function setKey(key) {
  await readResponse(await request("/api/browser-session", {
    method: "POST", headers: { Authorization: "Bearer " + key },
  }));
  authenticated = true;
  authPromise = undefined;
}
export async function clearKey() {
  await readResponse(await request("/api/browser-session", { method: "DELETE" }));
  authenticated = false;
  authPromise = undefined;
}

export class ApiError extends Error {
  constructor(message, status = 0, category = "network", retryAfter = null) {
    super(message);
    this.name = "ApiError";
    Object.assign(this, { status, category, retryAfter });
  }
}

export function isAuthError(error) { return error?.status === 401; }

export async function request(path, opts = {}) {
  try { return await fetch(path, opts); }
  catch (e) {
    if (e.name === "AbortError") throw e;
    throw new ApiError("Connection failed. Check your connection and retry.");
  }
}

// All response decoding lives here, including pages that post FormData or
// intentionally use public endpoints without a bearer credential.
export async function readResponse(res) {
  const ct = (res.headers.get("Content-Type") || "").toLowerCase();
  let body = null;
  let malformed = false;
  try {
    const text = await res.text();
    if (text) body = ct.includes("json") ? JSON.parse(text) : text;
  } catch (_) { malformed = true; }
  const status = res.status;
  if (status === 401) throw new ApiError("Authentication failed. Check your API key.", status, "auth");
  if (status === 403) {
    const detail = [body?.error?.message, body?.detail].find(value => typeof value === "string" && value.trim());
    throw new ApiError(detail || "Permission denied.", status, "permission");
  }
  if (status === 429) throw new ApiError("Too many requests. Wait and retry.", status, "rate-limit", res.headers.get("Retry-After"));
  if (status >= 500) throw new ApiError("The portal is having trouble. Please retry.", status, "server");
  if (!res.ok) throw new ApiError(body?.error?.message || body?.detail || `Request failed (HTTP ${status}).`, status, "request");
  if (malformed) throw new ApiError("The portal returned an invalid response. Please retry.", status, "response");
  return body;
}

export async function api(path, opts = {}) {
  const headers = new Headers(opts.headers || {});
  headers.set("Accept", "application/json");
  if (opts.body && !(opts.body instanceof FormData) && !headers.has("Content-Type")) headers.set("Content-Type", "application/json");
  return readResponse(await request(path, { ...opts, headers }));
}

export function showRequestError(error, target = null, retry = () => location.reload()) {
  if (isAuthError(error)) document.getElementById("auth-banner")?.classList.remove("hidden");
  let notice = target || document.getElementById("request-error");
  if (!notice) {
    notice = document.createElement("p");
    notice.id = "request-error";
    (document.querySelector("main") || document.body).prepend(notice);
  }
  notice.classList.remove("hidden");
  notice.setAttribute("role", "alert");
  notice.replaceChildren(document.createTextNode(error.message + " "));
  const button = document.createElement("button");
  button.type = "button";
  button.textContent = "Retry";
  button.className = "request-retry";
  button.addEventListener("click", retry);
  notice.append(button);
}

// Display lexicon (docs/design/portal-lexicon.md). Fetched once from
// /api/branding and cached. The static HTML is already substituted server-side;
// this is for labels the JS builds at runtime. Falls back to the nova-a wording
// (matching the Go-side defaults) if the fetch fails, so a label never renders
// blank or broken — display text is never load-bearing.
const LEX_DEFAULT = Object.freeze({
  product_name: "Zenoh Trial",
  product_title: "Zenoh Trial — Vendor Portal",
  org: "vendor",
  orgs: "vendors",
  org_title: "Vendor",
  orgs_title: "Vendors",
  panoscope_url: "",
});
let _lexCache = null;
let _lexPromise = null;
export async function lexicon() {
  if (_lexCache) return _lexCache;
  if (!_lexPromise) {
    _lexPromise = request("/api/branding", { headers: { Accept: "application/json" } })
      .then(readResponse)
      .catch((e) => { showRequestError(e); return LEX_DEFAULT; })
      .then((l) => (_lexCache = Object.assign({}, LEX_DEFAULT, l)));
  }
  return _lexPromise;
}

// Share the page's identity transition with page-specific authorization UI.
// Credential replacement establishes a server session before reloading.
let authPromise;
export function hydrateAuth() {
  if (!authPromise) authPromise = hydrateAuthOnce();
  return authPromise;
}

// Hydrate the auth banner + #who indicator on every page.
async function hydrateAuthOnce() {
  const who = document.getElementById("who");
  const banner = document.getElementById("auth-banner");
  const welcome = document.getElementById("welcome");
  const operatorControls = document.querySelectorAll(".operator-only, #operator-actions");
  operatorControls.forEach(el => el.classList.add("hidden"));
  try {
    // Probe a low-cost auth-required endpoint. /api/whoami is ~60 bytes and
    // carries visible_orgs, the only thing the header needed; this used to
    // fetch the whole /api/orgs listing (~63 KB) on every page load to render
    // one count. The lexicon fetch is independent, so both go at once rather
    // than one after the other.
    const [v, lex] = await Promise.all([api("/api/whoami"), lexicon()]);
    authenticated = true;
    if (banner) banner.classList.add("hidden");
    if (welcome) welcome.classList.add("hidden");
    const count = v.visible_orgs || 0;
    const noun = count === 1 ? lex.org : lex.orgs;
    if (who) who.textContent = `${count} ${noun} visible`;
    if (v.operator === true) operatorControls.forEach(el => el.classList.remove("hidden"));
    return v;
  } catch (e) {
    authenticated = false;
    if (banner) banner.classList.toggle("hidden", !isAuthError(e));
    if (welcome) welcome.classList.toggle("hidden", !isAuthError(e));
    if (who) who.textContent = isAuthError(e) ? "unauthenticated" : e.message;
    if (!isAuthError(e)) showRequestError(e);
    return null;
  }
}

// Tooltip wiring. Every <span class="tip" data-tip="..."> gets a click +
// hover behavior. Uses the data-tip attribute as the content and the
// browser's native title attribute as fallback (mobile / a11y).
function wireTooltips(root = document) {
  for (const el of root.querySelectorAll(".tip[data-tip]")) {
    const text = el.getAttribute("data-tip");
    el.setAttribute("title", text);
    el.setAttribute("role", "button");
    el.setAttribute("tabindex", "0");
    el.setAttribute("aria-label", text);
    el.addEventListener("click", (e) => {
      e.preventDefault();
      toggleTooltipBubble(el, text);
    });
    el.addEventListener("keydown", (e) => {
      if (e.key === "Escape") {
        el.querySelector(".tip-bubble")?.remove();
        e.preventDefault();
      }
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        toggleTooltipBubble(el, text);
      }
    });
  }
}

function toggleTooltipBubble(el, text) {
  // Remove any existing bubble first (toggle off).
  const existing = el.querySelector(".tip-bubble");
  if (existing) { existing.remove(); return; }
  // Close any other open bubbles on the page.
  for (const b of document.querySelectorAll(".tip-bubble")) b.remove();

  const bubble = document.createElement("span");
  bubble.className = "tip-bubble";
  bubble.textContent = text;
  // Append as a CHILD of the .tip span (which has position: relative)
  // so the bubble's position: absolute anchors to the icon itself, not
  // to a distant ancestor. Without this it flies to the page corner.
  el.appendChild(bubble);

  // Auto-dismiss on next outside click.
  setTimeout(() => {
    const dismiss = (ev) => {
      if (!bubble.contains(ev.target) && ev.target !== el) {
        bubble.remove();
        document.removeEventListener("click", dismiss);
      }
    };
    document.addEventListener("click", dismiss);
  }, 0);
}

// Copy-to-clipboard helper for any <button data-copy-target="elemId">.
function wireCopyButtons(root = document) {
  for (const btn of root.querySelectorAll(".copy-btn[data-copy-target]")) {
    btn.addEventListener("click", async (e) => {
      e.preventDefault();
      const id = btn.getAttribute("data-copy-target");
      const src = document.getElementById(id);
      if (!src) return;
      const text = src.innerText || src.textContent || "";
      try {
        await navigator.clipboard.writeText(text);
        const orig = btn.textContent;
        btn.textContent = "Copied";
        btn.disabled = true;
        setTimeout(() => { btn.textContent = orig; btn.disabled = false; }, 1500);
      } catch (_) {
        // Fallback: select the text node.
        const r = document.createRange();
        r.selectNodeContents(src);
        const sel = window.getSelection();
        sel.removeAllRanges();
        sel.addRange(r);
      }
    });
  }
}

// --- theme picker ----------------------------------------------------
//
// Three modes selectable from a three-segment header picker:
//   "system" (no data-theme attr; honors prefers-color-scheme)
//   "light"  (data-theme="light"; forces light regardless of OS)
//   "dark"   (data-theme="dark";  forces dark regardless of OS)
// Persisted in localStorage at `portalTheme`. Applied at module import
// time (before paint) to avoid flashing the wrong palette. Each segment
// jumps directly to its mode on click; we do not cycle a single button
// — clicking "Dark" when already on dark would otherwise look broken
// (no visible change, and the button's label would advance to the next
// state in the cycle).
//
// Active segment is marked via aria-pressed="true" and styled in CSS.

const THEME_KEY = "portalTheme";
const THEME_MODES = ["light", "system", "dark"];
const THEME_LABELS = { light: "Light", system: "System", dark: "Dark" };

function applyTheme(mode) {
  const root = document.documentElement;
  const resolved = effectiveTheme(mode);
  // Always set data-theme to the resolved value (never absent), so
  // attribute-keyed CSS overrides like [data-theme="dark"] .swagger-ui
  // fire correctly in "system" mode too. The picker still reads the
  // user's choice from localStorage, so System remains a distinct mode
  // visible in the UI even though it resolves to light/dark on <html>.
  root.setAttribute("data-theme", resolved);
  swapHljsTheme(resolved);
}

function effectiveTheme(mode) {
  if (mode === "light" || mode === "dark") return mode;
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function swapHljsTheme(eff) {
  const light = "/highlight/github.min.css";
  const dark = "/highlight/github-dark.min.css";
  const want = eff === "dark" ? dark : light;
  for (const link of document.querySelectorAll("link[rel=stylesheet]")) {
    if (link.href.endsWith(light) || link.href.endsWith(dark)) {
      if (!link.href.endsWith(want)) link.href = want;
    }
  }
}

function loadTheme() {
  try {
    const v = localStorage.getItem(THEME_KEY);
    return THEME_MODES.includes(v) ? v : "system";
  } catch (e) { return "system"; }
}

function selectTheme(mode) {
  if (!THEME_MODES.includes(mode)) return;
  try { localStorage.setItem(THEME_KEY, mode); } catch (e) {}
  applyTheme(mode);
  refreshThemePickerActive();
}

function refreshThemePickerActive() {
  const cur = loadTheme();
  for (const btn of document.querySelectorAll("#theme-toggle button[data-mode]")) {
    btn.setAttribute("aria-pressed", btn.dataset.mode === cur ? "true" : "false");
  }
}

// Wire up the three-segment theme picker. Each HTML page ships the picker
// as static markup inside <header> so the header grid is stable on first
// paint — the previous post-DOMContentLoaded insert caused a visible
// layout shift (perceived as a "flash" of the header re-laying out). If a
// page predates that markup we fall back to creating the widget here.
function installThemeToggle() {
  const header = document.querySelector("header");
  if (!header) return;
  let group = document.getElementById("theme-toggle");
  if (!group) {
    group = document.createElement("div");
    group.id = "theme-toggle";
    group.setAttribute("role", "group");
    group.setAttribute("aria-label", "Color theme");
    for (const mode of THEME_MODES) {
      const seg = document.createElement("button");
      seg.type = "button";
      seg.dataset.mode = mode;
      seg.textContent = THEME_LABELS[mode];
      seg.setAttribute("title", `Use ${THEME_LABELS[mode].toLowerCase()} theme`);
      seg.setAttribute("aria-pressed", "false");
      group.appendChild(seg);
    }
    const who = document.getElementById("who");
    if (who && who.parentElement === header) header.insertBefore(group, who);
    else header.appendChild(group);
  }
  for (const seg of group.querySelectorAll("button[data-mode]")) {
    if (!seg.dataset.wired) {
      seg.addEventListener("click", () => selectTheme(seg.dataset.mode));
      seg.dataset.wired = "1";
    }
  }
  refreshThemePickerActive();
}

// Apply persisted theme immediately on module load so the first paint
// uses the right palette. The button gets installed on DOMContentLoaded.
applyTheme(loadTheme());

if (window.matchMedia) {
  const mq = window.matchMedia("(prefers-color-scheme: dark)");
  // When mode is "system", react to OS theme changes live.
  mq.addEventListener?.("change", () => {
    if (loadTheme() === "system") applyTheme("system");
  });
}

document.addEventListener("DOMContentLoaded", installThemeToggle);

// Bind every existing key form before page-specific requests begin.
function wireKeyForm() {
  const saveBtn = document.getElementById("api-key-save");
  const input = document.getElementById("api-key-input");
  const signIn = async () => {
    const key = input?.value.trim();
    if (!key) return;
    try {
      await setKey(key);
      input.value = "";
      location.reload();
    } catch (error) { input.value = ""; showRequestError(error); }
  };
  if (saveBtn) { saveBtn.textContent = "Sign in"; saveBtn.addEventListener("click", signIn); }
  if (input) input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") signIn();
  });
}

// Home-page wiring.
async function initHome() {
  const loadingBody = document.querySelector("#vendors-table tbody");
  if (loadingBody) loadingBody.innerHTML = '<tr><td colspan="8" role="status">Loading slots…</td></tr>';
  wireTooltips();
  wireCopyButtons();
  const lex = await lexicon();

  const health = await request("/api/health").then(readResponse).catch((e) => { showRequestError(e); return null; });
  const healthEl = document.getElementById("health");
  if (health && healthEl) {
    const line = document.createElement("small");
    line.className = "hint";
    line.textContent = `portal v${health.version} • uptime ${formatUptime(health.uptime)}${health.sunset ? " • sunset " + health.sunset : ""}`;
    healthEl.replaceChildren(line);
  }
  const sunset = document.getElementById("sunset-banner");
  if (health && health.sunset && sunset) {
    sunset.classList.remove("hidden");
    sunset.innerHTML = `<p>${lex.product_name} sunset: <strong>${health.sunset}</strong></p>`;
  }

  const auth = await hydrateAuth();
  if (!auth) {
    if (loadingBody) loadingBody.innerHTML = '<tr><td colspan="8">Slots could not be loaded.</td></tr>';
    return;
  }

  // The index page is the one surface that renders the whole org table, so it
  // fetches the listing itself. hydrateAuth used to return it for everybody,
  // which made every other page pay ~63 KB for a header count it could get
  // from /api/whoami.
  let data;
  try { data = await api("/api/orgs"); }
  catch (e) {
    if (loadingBody) loadingBody.innerHTML = '<tr><td colspan="8">Slots could not be loaded.</td></tr>';
    showRequestError(e);
    return;
  }

  // Render vendors table or empty-state hint.
  const tbody = document.querySelector("#vendors-table tbody");
  const empty = document.getElementById("vendors-empty");
  const table = document.getElementById("vendors-table");
  // ADR 1052 Phase 3: prefer the `orgs` listing key, fall back to `vendors`.
  const rows = [...(data.orgs || data.vendors || [])].sort(compareDisplayNames);
  if (tbody) {
    tbody.innerHTML = "";
    if (rows.length === 0) {
      if (empty) empty.classList.remove("hidden");
      if (table) table.classList.add("hidden");
    } else {
      if (empty) empty.classList.add("hidden");
      if (table) table.classList.remove("hidden");
      for (const v of rows) {
        const tr = document.createElement("tr");
        if (v.synthetic) tr.classList.add("synthetic-row");
        const badge = v.synthetic
          ? ` <span class="synthetic-badge" title="Operator-managed demo / illustrative ${lex.org}. Panoscope's 'Hide synthetic' toggle filters these out of the visualization.">synthetic</span>`
          : "";
        // Column header is "Synthetic?" — this says whether the row is an
        // operator-managed demo org, nothing to do with affiliation.
        const synthetic = v.synthetic ? "yes — demo" : "no";
        tr.innerHTML = `
          <td><code>${escapeHTML(v.prefix)}</code>${badge}</td>
          <td>${escapeHTML(v.display_name || "")}</td>
          <td>${v.approved ? "yes" : "<em>pending</em>"}</td>
          <td>${synthetic}</td>
          <td>${new Date(v.created_at * 1000).toISOString().slice(0, 19) + "Z"}</td>
        `;
        tbody.appendChild(tr);
      }
    }
  }

  // hydrateAuth reveals the register form only for an operator key.
  const form = document.getElementById("register-form");
  const successCard = document.getElementById("register-success");
  const dismissBtn = document.getElementById("success-dismiss");
  if (dismissBtn) dismissBtn.addEventListener("click", () => {
    if (successCard) successCard.classList.add("hidden");
    // Reset the form so the next mint starts predictably.
    const pref = document.getElementById("reg-prefix");
    const disp = document.getElementById("reg-display");
    const synth = document.getElementById("reg-synthetic");
    if (pref) pref.value = "";
    if (disp) disp.value = "";
    if (synth) synth.checked = false;
    // Reload vendors table so the just-minted row shows up.
    location.reload();
  });

  if (form) form.addEventListener("submit", async (e) => {
    e.preventDefault();
    const btn = form.querySelector("button[type=submit]");
    if (btn) { btn.disabled = true; btn.textContent = "Minting…"; }
    try {
      const resp = await api("/api/orgs", {
        method: "POST",
        body: JSON.stringify({
          prefix: document.getElementById("reg-prefix").value.trim(),
          display_name: document.getElementById("reg-display").value.trim(),
          auto_approve: document.getElementById("reg-auto-approve").checked,
          synthetic: document.getElementById("reg-synthetic").checked,
        }),
      });
      renderRegisterSuccess(resp);
    } catch (err) {
      const out = document.getElementById("register-result");
      out.classList.remove("hidden");
      out.textContent = "Error: " + err.message;
    } finally {
      if (btn) { btn.disabled = false; btn.textContent = `Mint ${lex.org}`; }
    }
  });
}

function renderRegisterSuccess(resp) {
  const card = document.getElementById("register-success");
  if (!card) return;
  card.classList.remove("hidden");

  const set = (id, val) => {
    const el = document.getElementById(id);
    if (el) el.textContent = val || "";
  };
  set("success-prefix", resp.prefix);
  set("success-api-key", resp.api_key);
  set("success-uri-san", resp.cert_uri_san);

  const link = document.getElementById("success-bundle-link");
  if (link && resp.bundle_url) {
    link.href = resp.bundle_url;
    link.textContent = "Download bundle (.tar.gz)";
  }

  // Surface backend warnings (e.g. ACL render failure) as a yellow
  // callout inside the success card rather than burying them in JSON.
  const warn = document.getElementById("success-warning");
  if (warn) {
    if (resp.note_to_share && resp.note_to_share.length > 0) {
      warn.classList.remove("hidden");
      warn.innerHTML = `<strong>Operator note:</strong> ${escapeHTML(resp.note_to_share)}`;
    } else {
      warn.classList.add("hidden");
      warn.innerHTML = "";
    }
  }

  // Re-wire tooltips + copy buttons inside the success card now that
  // they're visible.
  wireTooltips(card);
  wireCopyButtons(card);

  card.scrollIntoView({ behavior: "smooth", block: "start" });
}

function escapeHTML(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#39;",
  }[c]));
}

// Page dispatch. Each HTML file imports this; the page-specific .js files
// (topics.js, inspector.js) layer on top for their respective screens.
if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", route);
} else {
  route();
}

function route() {
  wireKeyForm();
  const path = location.pathname;
  if (path === "/" || path === "/index.html") initHome();
  else {
    wireTooltips();
    wireCopyButtons();
    hydrateAuth();
  }
}

export { escapeHTML, wireTooltips, wireCopyButtons };
