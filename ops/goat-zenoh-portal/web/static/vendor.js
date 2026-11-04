import { readResponse, request, showRequestError } from "/app.js";
// goat-zenoh-portal — Slot catalog ("Catalog → Browse everything").
//
// The page answers one question for a visitor: what is published on this
// fabric, by whom, in what format, and where do I go to look at it? It is
// built around ONE search field (slot name, alias, prefix fragment or topic
// word → Enter opens it), filter chips that ARE the counts, a sticky slot rail,
// and one-line topic rows whose primary action is the Inspector.
//
// Data: exactly ONE request per page load, `GET /api/catalog?include=empty`
// (#1338). Every view on this page — search index, rail, chips, single-slot
// view — reads that one in-memory object. No `/api/orgs`, no per-topic
// fan-out, no request on a click (ADR 1123: client demand must never amplify
// into fabric load). `/api/branding` and `/api/siblings` are shared-shell
// reads the other pages also make; they are not catalog requests.
//
// URL contract (inbound, preserved for the Progress page and documented links):
//   ?vendor=<prefix>    select one slot (Block 94 canon; wins on conflict)
//   ?v=<prefix>         same, backwards-compat alias
//   ?v=__synthetic__    kind=synthetic (the old default scope)
//   ?v=__all__          kind=all
// Outbound, the page keeps `vendor=` plus its own view state (`q`, `kind`,
// `format`, `schema`, `published`, `empty`) so "Share view" reproduces the
// exact view.
//
// Honesty of the publish column: the per-topic `last_seen_at` field is never
// written, so there is NO per-topic
// "Last seen" column. The per-slot fact the audit tailer really records
// (`last_publish_at`, `publish_count`) is shown once per card header, or
// "no publish recorded yet". The slot alias is DISPLAY ONLY (ADR 1039): it
// is searched and shown, never used to compose a key or a link.
import { api, isAuthError, escapeHTML, hydrateAuth, hasAuth, lexicon } from "/app.js";

const SYNTH_KEY = "__synthetic__";
const ALL_KEY = "__all__";
const HOUR = 3600;
const DAY = 24 * HOUR;
const KIND_DEFAULT = "synthetic";

// Operator notes the registration path writes into descriptions. They are
// hidden on this page only (the stored text is untouched; the Topics page
// still shows it).
const BOILERPLATE_DESC = /^auto-registered\b/i;

// Display lexicon (docs/design/portal-lexicon.md). Seeded so a render that
// somehow runs pre-init shows a sensible word, never blank.
let LEX = { org: "vendor", orgs: "vendors", org_title: "Vendor", orgs_title: "Vendors" };

// Block 94 cross-surface deep-link sibling base URLs; an empty field
// suppresses that row button.
let siblings = { stethoscope: "", flashlight: "" };

// The catalog, indexed once.
let SLOTS = []; // [{prefix, display_name, alias, kind, synthetic, approved, last, count, topics:[...]}]
let TOTAL_ORGS = 0; // catalog.totals.orgs — the number the masthead badge also describes
let NOW = Math.floor(Date.now() / 1000);

// View state. `sel` is the selected slot prefix ("" = grouped view).
const st = { q: "", kind: KIND_DEFAULT, schema: false, published: false, empty: false, fmt: "", sel: "" };

const $ = (id) => document.getElementById(id);

// --- init ------------------------------------------------------------------

async function init() {
  $("cards").innerHTML = '<p role="status">Loading catalog…</p>';
  await hydrateAuth();
  LEX = await lexicon();
  await loadSiblings();

  let cat;
  try {
    cat = await api("/api/catalog?include=empty");
  } catch (e) {
    showAuthGate(e);
    return;
  }
  NOW = Math.floor(Date.now() / 1000);
  indexCatalog(cat);
  readURL();
  wire();
  render();
}

async function loadSiblings() {
  // Public endpoint, best-effort; a portal without sibling URLs configured
  // simply renders no diagnose / tail buttons.
  try {
    const resp = await request("/api/siblings", { headers: { Accept: "application/json" } });
    const got = await readResponse(resp);
      siblings = { stethoscope: got.stethoscope || "", flashlight: got.flashlight || "" };
  } catch (e) {
    showRequestError(e);
  }
}

function showAuthGate(err) {
  if (isAuthError(err)) $("auth-banner")?.classList.remove("hidden");
  $("cards").innerHTML = `<div class="empty">Could not load the catalog: ${escapeHTML(err.message)}</div>`;
  showRequestError(err, $("cards"));
  $("result").textContent = "";
}

function indexCatalog(cat) {
  TOTAL_ORGS = (cat.totals && cat.totals.orgs) || (cat.orgs || []).length;
  SLOTS = (cat.orgs || []).map((o) => {
    const topics = (o.topics || []).map((t) => ({
      key: t.key_pattern,
      tail: elideKey(o.prefix, t.key_pattern),
      format: t.format || "",
      description: t.description && !BOILERPLATE_DESC.test(t.description) ? t.description : "",
      schemas: [...(t.schemas || [])].sort((a, b) => a.version.localeCompare(b.version, undefined, { numeric: true })),
    }));
    topics.sort((a, b) => a.key.localeCompare(b.key));
    return {
      prefix: o.prefix,
      display_name: o.display_name || o.prefix,
      alias: o.alias || "",
      synthetic: !!o.synthetic,
      approved: !!o.approved,
      kind: o.synthetic ? "synthetic" : (o.approved ? "real" : "pending"),
      last: o.last_publish_at || null,
      count: o.publish_count || 0,
      short: o.prefix.slice(0, 8) + "…",
      topics,
    };
  });
  SLOTS.sort((a, b) => a.display_name.localeCompare(b.display_name) || a.prefix.localeCompare(b.prefix));
}

// A topic key that starts with its own slot prefix is shown from the part that
// carries meaning; the card header already shows the prefix once.
function elideKey(prefix, key) {
  return key.startsWith(prefix + "/") ? key.slice(prefix.length + 1) : key;
}

function readURL() {
  const p = new URLSearchParams(location.search);
  const vendor = p.get("vendor") || p.get("v") || "";
  if (vendor === SYNTH_KEY) st.kind = "synthetic";
  else if (vendor === ALL_KEY) st.kind = "all";
  else if (vendor && SLOTS.some((s) => s.prefix === vendor)) st.sel = vendor;
  if (p.get("kind")) st.kind = p.get("kind");
  if (!["all", "real", "synthetic", "pending"].includes(st.kind)) st.kind = KIND_DEFAULT;
  // The old default landing was "All synthetic"; keep it, but a goatnet with
  // no synthetic slots at all lands on everything rather than on nothing.
  if (st.kind === "synthetic" && !p.get("kind") && !SLOTS.some((s) => s.synthetic)) st.kind = "all";
  st.q = p.get("q") || "";
  st.fmt = p.get("format") || "";
  st.schema = p.get("schema") === "1";
  st.published = p.get("published") === "24h";
  st.empty = p.get("empty") === "1";
  if (st.q) $("q").value = st.q;
}

function writeURL() {
  const u = new URL(location.href);
  for (const k of ["vendor", "v", "q", "kind", "format", "schema", "published", "empty"]) u.searchParams.delete(k);
  if (st.sel) u.searchParams.set("vendor", st.sel);
  if (st.q) u.searchParams.set("q", st.q);
  if (st.kind !== KIND_DEFAULT) u.searchParams.set("kind", st.kind);
  if (st.fmt) u.searchParams.set("format", st.fmt);
  if (st.schema) u.searchParams.set("schema", "1");
  if (st.published) u.searchParams.set("published", "24h");
  if (st.empty) u.searchParams.set("empty", "1");
  history.replaceState(null, "", u.toString());
}

// --- filtering ---------------------------------------------------------------

function slotMatchesQ(s, q) {
  return s.display_name.toLowerCase().includes(q) || s.alias.toLowerCase().includes(q) || s.prefix.includes(q);
}
function topicMatchesQ(t, q) {
  return t.key.toLowerCase().includes(q) || t.description.toLowerCase().includes(q) || t.format.toLowerCase() === q;
}
function topicPasses(t) {
  if (st.schema && t.schemas.length === 0) return false;
  if (st.fmt && t.format !== st.fmt) return false;
  return true;
}

// Slots in the current view, each with the topics that pass the filters. A
// slot whose name matches the query keeps all its (filter-passing) topics; a
// slot whose name does not match keeps only the topics that match.
function visible() {
  const q = st.q.trim().toLowerCase();
  const out = [];
  for (const s of SLOTS) {
    if (!st.empty && st.kind !== "all" && s.kind !== st.kind) continue; // "Nothing declared" counts every kind
    if (st.published && !(s.last && NOW - s.last < DAY)) continue;
    const declared = s.topics.length > 0;
    if (st.empty ? declared : !declared) continue;
    const nameHit = !q || slotMatchesQ(s, q);
    const ts = s.topics.filter((t) => topicPasses(t) && (nameHit || topicMatchesQ(t, q)));
    if (declared && ts.length === 0) continue;
    if (!declared && !nameHit) continue;
    out.push({ s, ts });
  }
  return out;
}

// --- render ----------------------------------------------------------------

function render() {
  NOW = Math.floor(Date.now() / 1000);
  // One chip per wire format present in the current kind. A format the
  // current kind does not offer cannot stay selected: otherwise switching
  // kind leaves a filter active with no pressed chip on screen.
  const fmts = [...new Set(SLOTS.filter((s) => st.sel ? s.prefix === st.sel : (st.kind === "all" || s.kind === st.kind)).flatMap((s) => s.topics.map((t) => t.format)).filter(Boolean))].sort();
  if (st.fmt && !fmts.includes(st.fmt)) {
    if (st.sel) fmts.push(st.fmt), fmts.sort(); // keep it visible; the card explains the empty match
    else st.fmt = "";
  }
  const v = visible();
  const declared = SLOTS.filter((s) => s.topics.length > 0);
  const counts = { all: declared.length, empty: SLOTS.length - declared.length };
  for (const k of ["real", "synthetic", "pending"]) counts[k] = declared.filter((s) => s.kind === k).length;
  for (const b of document.querySelectorAll("[data-count]")) b.textContent = counts[b.dataset.count] ?? "";

  for (const b of document.querySelectorAll('.chip[data-f="kind"]')) b.setAttribute("aria-pressed", String(b.dataset.v === st.kind));
  document.querySelector('.chip[data-f="schema"]').setAttribute("aria-pressed", String(st.schema));
  document.querySelector('.chip[data-f="published"]').setAttribute("aria-pressed", String(st.published));
  document.querySelector('.chip[data-f="empty"]').setAttribute("aria-pressed", String(st.empty));

  $("fmtchips").innerHTML = fmts.map((f) =>
    `<button type="button" class="chip" data-f="fmt" data-v="${escapeHTML(f)}" aria-pressed="${st.fmt === f}"><span class="mono">${escapeHTML(f)}</span></button>`).join(" ");

  // A selected slot shows regardless of kind / query, but the topic filters
  // (format, has-schema) still apply inside it — "this slot, filtered to X".
  const shown = st.sel ? SLOTS.filter((s) => s.prefix === st.sel).map((s) => ({ s, ts: s.topics.filter(topicPasses) })) : v;
  const nt = shown.reduce((a, x) => a + x.ts.length, 0);
  $("result").textContent = `${shown.length} of ${TOTAL_ORGS} visible ${LEX.orgs} · ${nt} topic${nt === 1 ? "" : "s"}`;

  renderRail(v);
  renderCards(shown);
  writeURL();
}

function renderRail(v) {
  const items = [
    `<button type="button" class="ri ${st.sel ? "" : "sel"}" data-p=""><span class="n"><span>All in view</span><span>${v.length}</span></span></button>`,
    `<div class="hd">${escapeHTML(LEX.orgs_title)}</div>`,
  ];
  for (const { s, ts } of v) {
    const badge = s.kind !== "real" && st.kind === "all" ? `<span class="badge ${s.kind}">${s.kind}</span>` : "";
    items.push(`<button type="button" class="ri ${st.sel === s.prefix ? "sel" : ""}" data-p="${escapeHTML(s.prefix)}" title="${escapeHTML(s.prefix)}">
      <span class="n"><span>${escapeHTML(s.display_name)}</span><span>${ts.length} topic${ts.length === 1 ? "" : "s"}</span></span>
      <span class="p"><span class="mono">${escapeHTML(s.short)}</span>${badge}</span></button>`);
  }
  // A selected slot outside the current scope (deep link from the Progress
  // page, a search hit) still gets a rail item, marked selected, so the rail
  // never shows nothing selected while a card is on screen.
  const sel = st.sel && !v.some((x) => x.s.prefix === st.sel) ? SLOTS.find((s) => s.prefix === st.sel) : null;
  if (sel) {
    const n = sel.topics.filter(topicPasses).length;
    items.push(`<div class="hd">Selected, outside the current filters</div>
      <button type="button" class="ri sel" data-p="${escapeHTML(sel.prefix)}" title="${escapeHTML(sel.prefix)}">
      <span class="n"><span>${escapeHTML(sel.display_name)}</span><span>${n} topic${n === 1 ? "" : "s"}</span></span>
      <span class="p"><span class="mono">${escapeHTML(sel.short)}</span><span class="badge ${sel.kind}">${sel.kind}</span></span></button>`);
  }
  $("rail").innerHTML = items.join("");
}

function renderCards(shown) {
  const cards = $("cards");
  if (shown.length === 0) {
    const what = st.q.trim() ? `<strong>${escapeHTML(st.q.trim())}</strong>` : "these filters";
    const where = st.empty ? `${LEX.orgs} with nothing declared` : (st.kind === "all" ? `all ${LEX.orgs}` : `${st.kind} ${LEX.orgs}`);
    cards.innerHTML = `<div class="empty">Nothing matches ${what} among ${escapeHTML(where)}.<br>
      <button type="button" id="loosen">Show all ${escapeHTML(LEX.orgs)}</button> or clear the filters.</div>`;
    $("loosen").addEventListener("click", () => {
      Object.assign(st, { kind: "all", schema: false, published: false, empty: false, fmt: "", q: "", sel: "" });
      $("q").value = "";
      render();
    });
    return;
  }
  // Cards the reader collapsed stay collapsed across a re-render.
  const closed = new Set([...cards.querySelectorAll("details.card:not([open])")].map((d) => d.dataset.p));
  cards.innerHTML = shown.map(({ s, ts }) => renderCard(s, ts)).join("");
  for (const d of cards.querySelectorAll("details.card")) if (closed.has(d.dataset.p)) d.open = false;
}

function renderCard(s, ts) {
  const q = st.q.trim().toLowerCase();
  const badge = s.kind !== "real" ? `<span class="badge ${s.kind}">${s.kind}</span>` : "";
  const alias = s.alias && s.alias !== s.display_name ? `<span class="alias" title="Display alias (never a key or ACL subject)">${hl(s.alias, q)}</span>` : "";
  const lp = s.last
    ? `<span class="seen" title="Most recent publish under this ${escapeHTML(LEX.org)} as recorded by the fabric audit log (one-minute batches, so up to a minute behind)."><span class="dot ${dotClass(s.last)}"></span>Last publish ${rel(s.last)} · ${s.count.toLocaleString()} publish${s.count === 1 ? "" : "es"}</span>`
    : `<span class="seen" title="The fabric audit log has not recorded a publish under this ${escapeHTML(LEX.org)}."><span class="dot"></span>no publish recorded yet</span>`;
  const inspectAll = `/inspector.html?key=${encodeURIComponent(s.prefix + "/**")}`;
  const body = ts.length === 0
    ? (s.topics.length === 0
      ? `<div class="nothing">No topic declared yet. Register one from the <a href="/topics.html">Register a topic</a> page.</div>`
      : `<div class="nothing">None of this ${escapeHTML(LEX.org)}'s ${s.topics.length} topic${s.topics.length === 1 ? "" : "s"} matches the current filters.</div>`)
    : `<div class="table-scroll" tabindex="0" role="region" aria-label="${escapeHTML(s.display_name)} topics"><table><thead><tr><th>Topic</th><th>Format</th><th>Schema</th><th></th></tr></thead><tbody>${ts.map((t) => renderRow(s, t, q)).join("")}</tbody></table></div>`;
  return `<div class="card-shell"><details class="card" open data-p="${escapeHTML(s.prefix)}">
    <summary>
      <span class="tw" aria-hidden="true">▶</span>
      <span class="name">${hl(s.display_name, q)}</span>${badge}${alias}
      <span class="prefix mono">${hl(s.prefix, q)}</span>
      <span class="meta">${lp}<span>${ts.length} topic${ts.length === 1 ? "" : "s"}</span></span>
    </summary>
    ${body}
  </details>
    <div class="card-tools"><button type="button" class="cp" data-copy="${escapeHTML(s.prefix)}" title="Copy prefix" aria-label="Copy prefix">Copy prefix</button><a href="${inspectAll}" title="Open every topic of this ${escapeHTML(LEX.org)} in the Inspector">Inspect ${escapeHTML(LEX.org)}</a></div>
  </div>`;
}

function renderRow(s, t, q) {
  const inspectorHref = `/inspector.html?key=${encodeURIComponent(t.key)}`;
  const keyQS = `key=${encodeURIComponent(t.key)}`;
  const latest = t.schemas.length ? t.schemas[t.schemas.length - 1].version : null;
  const schemaHref = latest ? `/schema.html?${keyQS}&version=${encodeURIComponent(latest)}` : null;
  const versions = t.schemas.length
    ? t.schemas.map((v) => `<a class="vp" href="/schema.html?${keyQS}&version=${encodeURIComponent(v.version)}" title="uploaded ${new Date(v.uploaded_at * 1000).toISOString().slice(0, 10)} · sha256 ${escapeHTML(String(v.sha256).slice(0, 12))}…">${escapeHTML(v.version)}</a>`).join("")
    : `<span class="none">no schema</span>`;
  const elided = t.tail !== t.key;
  const acts = [
    schemaHref ? `<a href="${schemaHref}" title="Read the data dictionary">Schema</a>` : `<span class="dis" title="No schema registered">Schema</span>`,
  ];
  // Block 94 sibling buttons keep the `key=<topic>` contract and open in a new tab.
  if (siblings.stethoscope) acts.push(`<a class="sibling-link" href="${siblings.stethoscope}/?${keyQS}" target="_blank" rel="noopener noreferrer" title="Diagnose this topic in stethoscope">Diagnose ↗</a>`);
  if (siblings.flashlight) acts.push(`<a class="sibling-link" href="${siblings.flashlight}/?${keyQS}" target="_blank" rel="noopener noreferrer" title="Tail this topic in flashlight">Tail ↗</a>`);
  return `<tr class="t" tabindex="0" role="link" data-href="${inspectorHref}" aria-label="Open ${escapeHTML(t.key)} in the Inspector" title="${escapeHTML(t.key)}">
    <td class="k"><div class="key mono">${elided ? '<span class="el">…/</span>' : ""}<span class="tail">${hl(t.tail, q)}</span><button type="button" class="cp" data-copy="${escapeHTML(t.key)}" title="Copy full key" aria-label="Copy full key">⧉</button></div>${t.description ? `<div class="d">${hl(t.description, q)}</div>` : ""}</td>
    <td class="f"><span class="fmt">${escapeHTML(t.format)}</span></td>
    <td class="s">${versions}</td>
    <td class="act"><span class="acts">${acts.join("")}</span></td>
  </tr>`;
}

function rel(t) {
  const d = NOW - t;
  if (d < 60) return `${Math.max(d, 0)} s ago`;
  if (d < HOUR) return `${Math.round(d / 60)} min ago`;
  if (d < DAY) return `${Math.round(d / HOUR)} h ago`;
  return `${Math.round(d / DAY)} d ago`;
}
function dotClass(t) {
  const d = NOW - t;
  return d < 5 * 60 ? "ok" : d < DAY ? "am" : "old";
}
// Escape, then mark the first match of q. Output is safe HTML.
function hl(s, q) {
  s = String(s);
  const lo = s.toLowerCase();
  if (!q || lo.length !== s.length) return escapeHTML(s); // offsets drift when lowercasing changes length
  const i = lo.indexOf(q);
  if (i < 0) return escapeHTML(s);
  return escapeHTML(s.slice(0, i)) + "<mark>" + escapeHTML(s.slice(i, i + q.length)) + "</mark>" + escapeHTML(s.slice(i + q.length));
}

// --- typeahead -------------------------------------------------------------

let sug = [];
let si = 0;

function suggest() {
  const box = $("suggest");
  const input = $("q");
  const q = input.value.trim().toLowerCase();
  if (!q) { closeSuggest(); return; }
  const slots = SLOTS.filter((s) => slotMatchesQ(s, q)).slice(0, 6).map((s) => ({ t: "slot", s }));
  const topics = [];
  for (const s of SLOTS) for (const tp of s.topics) if (topicMatchesQ(tp, q)) topics.push({ t: "topic", s, tp });
  sug = [...slots, ...topics.slice(0, 8)];
  si = 0;
  let h = "";
  if (!sug.length) {
    h = `<div class="grp">No ${escapeHTML(LEX.org)} or topic matches “${escapeHTML(q)}”</div>`;
  } else {
    if (slots.length) {
      h += `<div class="grp">${escapeHTML(LEX.orgs_title)}</div>` + slots.map((x, i) =>
        `<div class="it ${i === 0 ? "sel" : ""}" role="option" data-i="${i}"><span>${hl(x.s.display_name, q)}</span>${x.s.alias && x.s.alias !== x.s.display_name ? `<span class="mono">${hl(x.s.alias, q)}</span>` : ""}<span class="mono">${hl(x.s.short, q)}</span><span class="r">${x.s.topics.length} topic${x.s.topics.length === 1 ? "" : "s"} · ${x.s.kind}</span></div>`).join("");
    }
    if (topics.length) {
      h += `<div class="grp">Topics</div>` + topics.slice(0, 8).map((x, j) => {
        const i = slots.length + j;
        return `<div class="it ${i === 0 ? "sel" : ""}" role="option" data-i="${i}"><span class="mono" title="${escapeHTML(x.tp.key)}">${hl(x.tp.tail, q)}</span><span class="r">${escapeHTML(x.s.display_name)} · ${escapeHTML(x.tp.format)}</span></div>`;
      }).join("");
    }
  }
  h += `<div class="it more" role="option" data-i="-1">Filter the page by “${escapeHTML(q)}” — Shift+Enter</div>`;
  box.innerHTML = h;
  box.classList.add("open");
  input.setAttribute("aria-expanded", "true");
}

function closeSuggest() {
  $("suggest").classList.remove("open");
  $("q").setAttribute("aria-expanded", "false");
  // A closed box has no list: Enter after re-focusing must never act on a
  // suggestion the user can no longer see.
  sug = [];
  si = 0;
}

function markSuggest() {
  for (const el of document.querySelectorAll(".suggest .it")) el.classList.toggle("sel", Number(el.dataset.i) === si);
}

// Enter on a suggestion. A slot selects it (the URL becomes ?vendor=<prefix>,
// exactly what the Progress page links to); a topic opens the Inspector at
// that key; "filter the page" applies the text as the page query.
function pick(i) {
  const list = sug;
  closeSuggest();
  if (!$("q").value.trim()) {
    // Enter on an empty field clears the query; never act on a stale list.
    st.q = "";
    render();
    return;
  }
  if (i === -1 || !list.length) {
    st.q = $("q").value.trim();
    st.sel = "";
    render();
    return;
  }
  const x = list[i];
  if (x.t === "slot") {
    st.q = "";
    $("q").value = "";
    st.sel = x.s.prefix;
    render();
    window.scrollTo({ top: 0 });
    return;
  }
  location.href = `/inspector.html?key=${encodeURIComponent(x.tp.key)}`;
}

// --- wiring ----------------------------------------------------------------

function wire() {
  const input = $("q");
  input.addEventListener("input", suggest);
  input.addEventListener("keydown", (e) => {
    const open = $("suggest").classList.contains("open");
    if (e.key === "ArrowDown" && open) { e.preventDefault(); si = Math.min(si + 1, sug.length - 1); markSuggest(); }
    else if (e.key === "ArrowUp" && open) { e.preventDefault(); si = Math.max(si - 1, 0); markSuggest(); }
    else if (e.key === "Enter") { e.preventDefault(); pick(e.shiftKey || !open ? -1 : si); }
    else if (e.key === "Escape") { if (!input.value && !st.q) st.sel = ""; input.value = ""; st.q = ""; closeSuggest(); render(); }
  });
  $("suggest").addEventListener("mousedown", (e) => {
    const it = e.target.closest(".it[data-i]");
    if (it) { e.preventDefault(); pick(Number(it.dataset.i)); }
  });
  document.addEventListener("click", (e) => { if (!e.target.closest(".searchwrap")) closeSuggest(); });

  $("chips").addEventListener("click", (e) => {
    const b = e.target.closest(".chip");
    if (!b) return;
    const f = b.dataset.f;
    const v = b.dataset.v;
    if (f === "kind") { st.kind = v; st.sel = ""; st.empty = false; }
    else if (f === "schema") st.schema = !st.schema;
    else if (f === "published") st.published = !st.published;
    else if (f === "empty") { st.empty = !st.empty; st.sel = ""; }
    else if (f === "fmt") st.fmt = st.fmt === v ? "" : v;
    render();
  });

  $("rail").addEventListener("click", (e) => {
    const r = e.target.closest(".ri");
    if (!r) return;
    st.sel = r.dataset.p || "";
    render();
    window.scrollTo({ top: 0 });
  });

  $("cards").addEventListener("click", (e) => {
    const cp = e.target.closest(".cp");
    if (cp) { e.preventDefault(); e.stopPropagation(); copy(cp.dataset.copy, cp); return; }
    if (e.target.closest("a, summary")) return;
    const tr = e.target.closest("tr.t");
    if (tr) location.href = tr.dataset.href;
  });

  $("cards").addEventListener("keydown", (e) => {
    const tag = document.activeElement ? document.activeElement.tagName : "";
    const typing = tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT";
    if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey || document.activeElement?.isContentEditable) return;
    if (e.key === "/" && !typing) { e.preventDefault(); input.focus(); input.select(); return; }
    if (typing) return;
    // Rows inside a collapsed card are not rendered; focusing one is a no-op
    // and the cursor would never move.
    const rows = [...document.querySelectorAll("tr.t")].filter((r) => !r.closest("details:not([open])"));
    const i = rows.indexOf(document.activeElement);
    if (e.key === "j") { e.preventDefault(); (rows[i + 1] || rows[0])?.focus(); }
    else if (e.key === "k") { e.preventDefault(); (rows[i - 1] || rows[rows.length - 1])?.focus(); }
    else if (e.key === "Enter" && i >= 0) { location.href = rows[i].dataset.href; }
  });

  $("share").addEventListener("click", () => copy(location.href, null, "Link to this view copied"));

  // Sticky offsets (rail, card headers) follow the toolbar's real height,
  // which changes as chips wrap.
  const toolbar = $("toolbar");
  const setH = () => document.querySelector("main").style.setProperty("--toolbar-h", `${toolbar.offsetHeight + 8}px`);
  setH();
  if (window.ResizeObserver) new ResizeObserver(setH).observe(toolbar);
}

async function copy(text, btn, msg) {
  try {
    await navigator.clipboard.writeText(text);
  } catch (e) {
    toast("Copy failed — select the text instead");
    return;
  }
  if (btn) {
    const label = btn.dataset.copyLabel ??= btn.textContent;
    btn.classList.add("done");
    btn.textContent = "✓";
    setTimeout(() => { btn.classList.remove("done"); btn.textContent = label; }, 1200);
  }
  toast(msg || `Copied ${text.length > 48 ? text.slice(0, 48) + "…" : text}`);
}

let toastTimer;
function toast(m) {
  const t = $("toast");
  t.textContent = m;
  t.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.classList.remove("show"), 2200);
}

if (!hasAuth()) {
  $("auth-banner")?.classList.remove("hidden");
}
init();
