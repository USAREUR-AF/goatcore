// goat-zenoh-portal — schema viewer.
//
// Fetches GET /api/schemas/render/<key>[?version=v2] and renders the
// returned descriptor tree as a Swagger-style read-only page.
import { api, escapeHTML, hydrateAuth, hasAuth, lexicon } from "/app.js";

import { schemaDownloadLabel } from "/display.js";

let cachedRendered = null;

// Display lexicon (docs/design/portal-lexicon.md). Module-level; set by init.
// NB: the "vendor" schema-source enum value (source === "vendor") is a machine
// field from the API, NOT a lexicon term — it stays "vendor" everywhere.
let LEX = { org: "vendor", orgs: "vendors", org_title: "Vendor", orgs_title: "Vendors" };

async function init() {
  await hydrateAuth();
  LEX = await lexicon();
  const input = document.getElementById("key-expr");
  const versionSel = document.getElementById("version-select");
  const btn = document.getElementById("load-schema");
  document.getElementById("m-download").addEventListener("click", e => openSchemaResource(e, false));
  document.getElementById("m-json").addEventListener("click", e => openSchemaResource(e, true));

  // Prefill from ?key= and ?version=.
  const params = new URLSearchParams(location.search);
  if (params.get("key")) input.value = params.get("key");
  if (params.get("version")) versionSel.value = params.get("version");

  btn.addEventListener("click", () => loadAndRender());
  input.addEventListener("keydown", (e) => { if (e.key === "Enter") loadAndRender(); });
  versionSel.addEventListener("change", () => loadAndRender(versionSel.value));

  if (input.value) await loadAndRender(versionSel.value);
}

async function loadAndRender(version) {
  const key = document.getElementById("key-expr").value.trim();
  const errEl = document.getElementById("error");
  errEl.classList.add("hidden");
  errEl.textContent = "";
  if (!key) { showError("Enter a topic key."); return; }
  const path = "/api/schemas/render/" + key + (version ? "?version=" + encodeURIComponent(version) : "");
  try {
    const data = await api(path);
    cachedRendered = data;
    renderHeader(data);
    populateVersionPicker(data, version);
    // The data dictionary + code tabs render identically for protobuf and
    // XSD (same JSON shape); source_lang only relabels the type/source
    // vocabulary so an XSD topic reads as elements/types, not messages.
    const isXSD = data.source_lang === "xsd";
    const msgH2 = document.querySelector("#messages-section h2");
    if (msgH2) msgH2.textContent = isXSD ? "Elements & types" : "Messages";
    const enumH2 = document.querySelector("#enums-section h2");
    if (enumH2) enumH2.textContent = isXSD ? "Enumerations" : "Enums";
    renderMessages(data.rendered, data.primary_message);
    renderEnums(data.rendered);
    renderProtoSource(data.proto_source, data.source_lang);
    renderExamples(data.examples || []);
    // Update URL so the view is shareable.
    const u = new URL(location.href);
    u.searchParams.set("key", key);
    if (data.schema && data.schema.version) u.searchParams.set("version", data.schema.version);
    history.replaceState(null, "", u.toString());
  } catch (e) {
    document.getElementById("meta").classList.add("hidden");
    document.getElementById("messages-section").classList.add("hidden");
    document.getElementById("enums-section").classList.add("hidden");
    document.getElementById("source-section").classList.add("hidden");
    document.getElementById("examples-section").classList.add("hidden");
    document.getElementById("empty-hint").classList.toggle("hidden",
      !(e.message && /no schema/i.test(e.message)));
    showError(e.message);
  }
}

function showError(msg) {
  const errEl = document.getElementById("error");
  errEl.textContent = msg;
  errEl.classList.remove("hidden");
}

function renderHeader(data) {
  document.getElementById("meta").classList.remove("hidden");
  document.getElementById("messages-section").classList.remove("hidden");
  document.getElementById("enums-section").classList.remove("hidden");
  document.getElementById("empty-hint").classList.add("hidden");
  const s = data.schema || {};
  document.getElementById("m-key").innerHTML = `<code>${escapeHTML(data.key)}</code>`;
  document.getElementById("m-vendor").textContent = data.vendor || "";
  document.getElementById("m-format").textContent = data.format || "";
  document.getElementById("m-version").textContent = s.version || "";
  document.getElementById("m-compat").textContent = s.compatibility || "";
  document.getElementById("m-sha").textContent = s.sha256 || "";
  document.getElementById("m-size").textContent = (s.size_bytes != null) ? (s.size_bytes + " bytes") : "";
  document.getElementById("m-uploaded").textContent =
    s.uploaded_at ? new Date(s.uploaded_at * 1000).toISOString().slice(0, 19) + "Z by " + (s.uploaded_by || "?") : "";
  const filesUl = document.getElementById("m-files");
  filesUl.innerHTML = "";
  for (const f of (data.rendered.files || [])) {
    const li = document.createElement("li");
    li.innerHTML = `<code>${escapeHTML(f.name)}</code> &mdash; package <code>${escapeHTML(f.package || "(none)")}</code> &mdash; ${escapeHTML(f.syntax || "")}`;
    filesUl.appendChild(li);
  }
  const dl = document.getElementById("m-download");
  dl.textContent = schemaDownloadLabel(data);
  dl.href = "/api/schemas/" + data.key + (s.version ? "?version=" + encodeURIComponent(s.version) : "");
  const jsonLink = document.getElementById("m-json");
  jsonLink.href = "/api/schemas/render/" + data.key + (s.version ? "?version=" + encodeURIComponent(s.version) : "");
  jsonLink.target = "_blank";
}

// The server owns attachment naming. Accept its sanitized filename; if the
// header is absent or unsafe, mirror attachmentFor's full-key naming contract.
function schemaDownloadFilename(response, data) {
  const disposition = response.headers.get("Content-Disposition") || "";
  const match = /(?:^|;)\s*filename\s*=\s*(?:"([^"]*)"|([^;\s]*))/i.exec(disposition);
  const filename = match?.[1] ?? match?.[2];
  if (filename && /^[A-Za-z0-9._-]+$/.test(filename) && filename !== "." && filename !== "..") return filename;
  const clean = value => String(value || "").replace(/[^A-Za-z0-9._-]/gu, "_");
  const format = data.schema?.format_storage;
  const extension = format === "protobuf-fds" ? "fds" : format === "xsd" ? "xsd" : "bin";
  return `${clean(data.key) || "schema"}-${clean(data.schema?.version)}.${extension}`;
}

// Plain navigation cannot attach the saved bearer. Keep both controls on the
// same authenticated read path and render raw JSON as text, never as markup.
async function openSchemaResource(event, rawJSON) {
  event.preventDefault();
  const link = event.currentTarget;
  if (link.getAttribute("aria-disabled") === "true") return;
  const key = hasAuth();
  if (!key) { showError("Sign in first."); return; }
  const data = cachedRendered;
  const popup = rawJSON ? window.open("about:blank", "_blank") : null;
  if (rawJSON && !popup) { showError("Allow a new tab to open raw JSON."); return; }
  if (popup) popup.opener = null;
  link.setAttribute("aria-disabled", "true");
  document.getElementById("error").classList.add("hidden");
  try {
    const response = await fetch(link.href, {
      headers: {},
      redirect: "error",
    });
    if (!response.ok) throw new Error(`Schema request failed: HTTP ${response.status}.`);
    if (rawJSON) {
      const body = await response.json();
      if (!popup.closed) {
        popup.document.title = "Schema JSON";
        const pre = popup.document.createElement("pre");
        pre.textContent = JSON.stringify(body, null, 2);
        popup.document.body.replaceChildren(pre);
      }
    } else {
      const blob = await response.blob();
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = schemaDownloadFilename(response, data);
      document.body.appendChild(anchor);
      try { anchor.click(); }
      finally { setTimeout(() => { URL.revokeObjectURL(url); anchor.remove(); }, 0); }
    }
  } catch (error) {
    if (popup && !popup.closed) popup.close();
    showError(error.message);
  } finally {
    link.removeAttribute("aria-disabled");
  }
}

function populateVersionPicker(data, currentVersion) {
  const sel = document.getElementById("version-select");
  const versions = (data.schema && data.schema.all_versions) || [];
  // Preserve "(latest)" sentinel + dedupe.
  sel.innerHTML = '<option value="">(latest)</option>';
  for (const v of versions) {
    const opt = document.createElement("option");
    opt.value = v;
    opt.textContent = v;
    sel.appendChild(opt);
  }
  if (currentVersion) {
    sel.value = currentVersion;
  } else if (data.schema && data.schema.version) {
    // Reflect the resolved-latest in the picker, but keep the value "" so
    // changing back to (latest) is a no-op.
    sel.value = "";
  }
}

function renderMessages(rendered, primaryFullName) {
  const out = document.getElementById("messages");
  out.innerHTML = "";
  const messages = (rendered && rendered.messages) || [];
  if (messages.length === 0) {
    out.innerHTML = '<p class="empty-hint">No messages in this descriptor set.</p>';
    return;
  }
  // Resolve "primary" by trying full-name first, then bare name match.
  let primary = null;
  if (primaryFullName) {
    primary = messages.find((m) => m.full_name === primaryFullName || m.name === primaryFullName) || null;
  }
  const ordered = [];
  if (primary) ordered.push(primary);
  for (const m of messages) if (m !== primary) ordered.push(m);
  for (const m of ordered) {
    out.appendChild(renderMessageCard(m, m === primary));
  }
}

function renderMessageCard(m, isPrimary) {
  const card = document.createElement("section");
  card.className = "msg-card" + (isPrimary ? " primary" : "");
  card.id = "msg-" + m.full_name;
  const head = document.createElement("h3");
  head.innerHTML = `${escapeHTML(m.name)}<span class="pkg">${escapeHTML(m.full_name)}${isPrimary ? " &nbsp;(primary)" : ""}</span>`;
  card.appendChild(head);

  if ((m.fields || []).length === 0) {
    const p = document.createElement("p");
    p.className = "card-rule";
    p.textContent = "(empty message — no fields)";
    card.appendChild(p);
  } else {
    const table = document.createElement("table");
    table.className = "field-table";
    table.innerHTML = `
      <thead>
        <tr><th>#</th><th>Name</th><th>Type</th><th>Cardinality</th><th>JSON name</th></tr>
      </thead>
      <tbody></tbody>`;
    const tbody = table.querySelector("tbody");
    for (const f of m.fields) {
      const tr = document.createElement("tr");
      tr.innerHTML = `
        <td class="num">${f.number}</td>
        <td class="name">${escapeHTML(f.name)}${f.oneof ? `<span class="pill oneof">oneof ${escapeHTML(f.oneof)}</span>` : ""}</td>
        <td class="type">${renderFieldType(f)}</td>
        <td>${renderCardinality(f)}</td>
        <td class="type">${escapeHTML(f.json_name || f.name)}</td>`;
      tbody.appendChild(tr);
    }
    card.appendChild(table);
  }
  if ((m.oneofs || []).length > 0) {
    const oneofP = document.createElement("p");
    oneofP.className = "card-rule";
    oneofP.innerHTML = "Oneofs: " + m.oneofs.map((o) =>
      `<code>${escapeHTML(o.name)}</code> [${o.fields.map(escapeHTML).join(", ")}]`).join(" &middot; ");
    card.appendChild(oneofP);
  }
  if ((m.reserved || []).length > 0) {
    const r = document.createElement("p");
    r.className = "card-rule";
    r.innerHTML = "Reserved: " + m.reserved.map((x) =>
      x.kind === "range" ? `${x.start}..${x.end}` : `<code>${escapeHTML(x.name)}</code>`).join(", ");
    card.appendChild(r);
  }
  // Wire type-ref jumps after the table is in the DOM.
  setTimeout(() => wireTypeRefs(card), 0);
  return card;
}

function renderFieldType(f) {
  if (f.cardinality === "map") {
    const k = f.map_key ? escapeHTML(f.map_key.type) : "?";
    const v = f.map_value ? renderTypeRefHTML(f.map_value.type, f.map_value.type_ref) : "?";
    return `map&lt;${k}, ${v}&gt;`;
  }
  return renderTypeRefHTML(f.type, f.type_ref);
}

function renderTypeRefHTML(type, ref) {
  if (ref) {
    return `<span class="type-ref" data-ref="${escapeHTML(ref)}">${escapeHTML(ref)}</span>`;
  }
  return escapeHTML(type);
}

function renderCardinality(f) {
  const c = f.cardinality || "";
  let pill = "";
  if (c === "repeated") pill = '<span class="pill repeated">repeated</span>';
  else if (c === "map") pill = '<span class="pill map">map</span>';
  else if (c === "optional") pill = '<span class="pill optional">optional</span>';
  else pill = `<span class="pill">${escapeHTML(c)}</span>`;
  return pill;
}

function wireTypeRefs(scope) {
  for (const el of scope.querySelectorAll(".type-ref")) {
    el.addEventListener("click", () => {
      const ref = el.getAttribute("data-ref");
      // Try message anchor first.
      const target = document.getElementById("msg-" + ref) || document.getElementById("enum-" + ref);
      if (target) target.scrollIntoView({ behavior: "smooth", block: "start" });
    });
  }
}

function renderProtoSource(protoSource, sourceLang) {
  const section = document.getElementById("source-section");
  const container = document.getElementById("source-blocks");
  container.innerHTML = "";
  if (!protoSource || Object.keys(protoSource).length === 0) {
    section.classList.add("hidden");
    return;
  }
  const isXSD = sourceLang === "xsd";
  const h2 = section.querySelector("h2");
  if (h2) h2.textContent = isXSD ? "XSD source" : ".proto source";
  const hlClass = isXSD ? "language-xml" : "language-protobuf";
  section.classList.remove("hidden");
  for (const fname of Object.keys(protoSource).sort()) {
    const label = document.createElement("p");
    label.className = "src-file";
    label.textContent = fname;
    container.appendChild(label);
    const pre = document.createElement("pre");
    pre.className = "code-block";
    const code = document.createElement("code");
    code.className = hlClass;
    code.textContent = protoSource[fname];
    pre.appendChild(code);
    container.appendChild(pre);
    if (window.hljs) window.hljs.highlightElement(code);
  }
}

function renderExamples(examples) {
  const section = document.getElementById("examples-section");
  const tabs = document.getElementById("ex-tabs");
  const panels = document.getElementById("ex-panels");
  tabs.innerHTML = "";
  panels.innerHTML = "";
  if (!examples || examples.length === 0) {
    section.classList.add("hidden");
    return;
  }
  section.classList.remove("hidden");
  examples.forEach((ex, idx) => {
    const btn = document.createElement("button");
    btn.className = "tab-btn" + (idx === 0 ? " active" : "");
    btn.dataset.idx = String(idx);
    btn.textContent = ex.display_name;
    btn.addEventListener("click", () => activateExampleTab(idx));
    tabs.appendChild(btn);
    panels.appendChild(buildExamplePanel(ex, idx));
  });
}

function buildExamplePanel(ex, idx) {
  const panel = document.createElement("div");
  panel.className = "tab-panel" + (idx === 0 ? " active" : "");
  panel.dataset.idx = String(idx);
  panel.dataset.lang = ex.language;
  renderExamplePanelView(panel, ex);
  return panel;
}

// View mode — read-only code block with Copy + Edit/Delete affordances.
function renderExamplePanelView(panel, ex) {
  panel.innerHTML = "";
  const meta = document.createElement("div");
  meta.className = "ex-meta";
  const source = ex.source || "autogen";
  // ADR 1052 Phase 3: the override provenance value is "org" (was "vendor").
  // Accept both so a stale cached page badges a fresh portal correctly.
  const overridden = source === "org" || source === "vendor";
  const badge = document.createElement("span");
  badge.className = "badge " + (overridden ? "vendor" : "autogen");
  badge.textContent = overridden ? `${LEX.org_title}-provided` : "Autogen";
  meta.appendChild(badge);

  const copyBtn = document.createElement("button");
  copyBtn.className = "ex-copy-btn";
  copyBtn.textContent = "Copy";
  copyBtn.addEventListener("click", () => copyToClipboard(ex.code, copyBtn));
  meta.appendChild(copyBtn);

  // Edit + revert affordances visible when there's a cached topic id
  // we can PUT/DELETE against (i.e. authenticated + the render
  // endpoint surfaced a topic_id, true on any v1 fetch).
  if (cachedRendered && cachedRendered.topic_id) {
    const editBtn = document.createElement("button");
    editBtn.className = "ex-copy-btn";
    editBtn.textContent = overridden ? "Edit override" : "Provide custom";
    editBtn.addEventListener("click", () => renderExamplePanelEdit(panel, ex));
    meta.appendChild(editBtn);

    if (overridden) {
      const revertBtn = document.createElement("button");
      revertBtn.className = "ex-copy-btn";
      revertBtn.textContent = "Revert to autogen";
      revertBtn.addEventListener("click", () => deleteVendorOverride(panel, ex));
      meta.appendChild(revertBtn);
    }
  }
  panel.appendChild(meta);

  const pre = document.createElement("pre");
  pre.className = "code-block";
  const code = document.createElement("code");
  code.className = "language-" + (ex.highlight_tag || ex.language || "plaintext");
  code.textContent = ex.code;
  pre.appendChild(code);
  panel.appendChild(pre);
  if (window.hljs) window.hljs.highlightElement(code);
}

// Edit mode — textarea + Save / Cancel buttons.
function renderExamplePanelEdit(panel, ex) {
  panel.innerHTML = "";
  const ta = document.createElement("textarea");
  ta.className = "ex-editor";
  ta.value = ex.code;
  ta.spellcheck = false;
  panel.appendChild(ta);

  const actions = document.createElement("div");
  actions.className = "ex-actions";
  const status = document.createElement("span");
  status.className = "status";
  status.textContent = `${LEX.org_title} override — saved bytes replace the autogen for this language.`;
  actions.appendChild(status);
  const spacer = document.createElement("span");
  spacer.className = "spacer";
  actions.appendChild(spacer);

  const cancelBtn = document.createElement("button");
  cancelBtn.textContent = "Cancel";
  cancelBtn.addEventListener("click", () => renderExamplePanelView(panel, ex));
  actions.appendChild(cancelBtn);

  const saveBtn = document.createElement("button");
  saveBtn.className = "primary";
  saveBtn.textContent = "Save";
  saveBtn.addEventListener("click", () => saveVendorOverride(panel, ex, ta, status, saveBtn, cancelBtn));
  actions.appendChild(saveBtn);

  panel.appendChild(actions);
  ta.focus();
}

async function saveVendorOverride(panel, ex, ta, status, saveBtn, cancelBtn) {
  const code = ta.value;
  if (!code.trim()) {
    status.textContent = "Cannot save an empty override.";
    return;
  }
  if (code.length > 64 * 1024) {
    status.textContent = `Too large — ${code.length} bytes exceeds the 64 KiB cap.`;
    return;
  }
  saveBtn.disabled = true;
  cancelBtn.disabled = true;
  status.textContent = "Saving…";
  try {
    const data = cachedRendered;
    const url = `/api/orgs/${encodeURIComponent(data.vendor)}/topics/${data.topic_id}/examples/${encodeURIComponent(ex.language)}`;
    const resp = await api(url, { method: "PUT", body: JSON.stringify({ code }) });
    // Replace ex in cachedRendered.examples so subsequent re-renders see the update.
    // ADR 1052 Phase 3: optimistic source matches the server's "org" provenance.
    for (const e of cachedRendered.examples) {
      if (e.language === ex.language) { e.code = resp.code; e.source = "org"; }
    }
    renderExamplePanelView(panel, { ...ex, code: resp.code, source: "org" });
  } catch (e) {
    status.textContent = "Save failed: " + e.message;
    saveBtn.disabled = false;
    cancelBtn.disabled = false;
  }
}

async function deleteVendorOverride(panel, ex) {
  if (!confirm(`Revert the ${ex.display_name} example to autogen? The ${LEX.org}-provided version will be deleted.`)) return;
  try {
    const data = cachedRendered;
    const url = `/api/orgs/${encodeURIComponent(data.vendor)}/topics/${data.topic_id}/examples/${encodeURIComponent(ex.language)}`;
    await api(url, { method: "DELETE" });
    // Reload to refresh the autogen body.
    await loadAndRender(data.schema && data.schema.version);
  } catch (e) {
    alert("Revert failed: " + e.message);
  }
}

function activateExampleTab(idx) {
  for (const el of document.querySelectorAll("#ex-tabs .tab-btn")) {
    el.classList.toggle("active", el.dataset.idx === String(idx));
  }
  for (const el of document.querySelectorAll("#ex-panels .tab-panel")) {
    el.classList.toggle("active", el.dataset.idx === String(idx));
  }
}

async function copyToClipboard(text, btn) {
  try {
    await navigator.clipboard.writeText(text);
    const orig = btn.textContent;
    btn.textContent = "Copied";
    setTimeout(() => { btn.textContent = orig; }, 1200);
  } catch (e) {
    btn.textContent = "Press Ctrl-C";
  }
}

function renderEnums(rendered) {
  const section = document.getElementById("enums-section");
  const out = document.getElementById("enums");
  out.innerHTML = "";
  const enums = (rendered && rendered.enums) || [];
  if (enums.length === 0) {
    section.classList.add("hidden");
    return;
  }
  section.classList.remove("hidden");
  for (const e of enums) {
    const card = document.createElement("section");
    card.className = "enum-card";
    card.id = "enum-" + e.full_name;
    const ul = (e.values || []).map((v) =>
      `<li><code>${escapeHTML(v.name)}</code> = ${v.number}</li>`).join("");
    card.innerHTML = `
      <h3>${escapeHTML(e.name)}<span class="pkg">${escapeHTML(e.full_name)}</span></h3>
      <ul>${ul}</ul>`;
    out.appendChild(card);
  }
}

if (!hasAuth()) {
  document.getElementById("auth-banner")?.classList.remove("hidden");
}
init();
