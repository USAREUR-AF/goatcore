import { readResponse, request, showRequestError } from "/app.js";
// goat-zenoh-portal — vendor topics screen.
import { api, escapeHTML, hasAuth, lexicon } from "/app.js";

// Display lexicon (docs/design/portal-lexicon.md). Module-level; set by
// initTopics before any handler that reads it runs. nova-a fallback seed.
let LEX = { org: "vendor", orgs: "vendors", org_title: "Vendor", orgs_title: "Vendors" };

async function initTopics() {
  LEX = await lexicon();
  const select = document.getElementById("vendor-select");
  if (!select) return;
  let vendors = [];
  try {
    // ADR 1052 Phase 3: prefer the `orgs` listing key, fall back to `vendors`.
    const resp = await api("/api/orgs");
    vendors = resp.orgs || resp.vendors || [];
  } catch (e) {
    showRequestError(e);
    return;
  }
  select.innerHTML = "";
  for (const v of vendors) {
    const opt = document.createElement("option");
    opt.value = v.prefix;
    const tag = v.synthetic ? " (synthetic)" : (v.approved ? "" : " (pending)");
    opt.textContent = v.prefix + tag;
    select.appendChild(opt);
  }
  select.addEventListener("change", refreshTopics);
  document.getElementById("refresh-topics").addEventListener("click", refreshTopics);
  document.getElementById("create-topic-form").addEventListener("submit", createTopic);

  // Reveal the protobuf message-name input only when the protobuf format
  // is selected. The dropdown value is the bare "protobuf:" prefix — the
  // message name comes from this field, so a vendor can't submit the old
  // "protobuf:MyMessage" placeholder by leaving the dropdown at a default
  // (the DRS / F-162 footgun).
  const formatSel = document.getElementById("t-format");
  formatSel.addEventListener("change", () => {
    const isProto = formatSel.value.startsWith("protobuf:");
    const isXMLBound = formatSel.value === "xml:";
    document.getElementById("t-format-msg-row").classList.toggle("hidden", !isProto);
    document.getElementById("t-format-el-row").classList.toggle("hidden", !isXMLBound);
  });

  // Bulk export/import wiring.
  document.getElementById("export-yaml").addEventListener("click", () => exportTopics("yaml"));
  document.getElementById("export-json").addEventListener("click", () => exportTopics("json"));
  document.getElementById("import-open").addEventListener("click", () => {
    document.getElementById("bulk-import").classList.remove("hidden");
  });
  document.getElementById("import-close").addEventListener("click", () => {
    document.getElementById("bulk-import").classList.add("hidden");
    document.getElementById("bulk-result").classList.add("hidden");
    document.getElementById("bulk-file").value = "";
  });
  document.getElementById("bulk-preview").addEventListener("click", () => bulkImport(true));
  document.getElementById("bulk-apply").addEventListener("click", () => bulkImport(false));

  // Schema upload dialog wiring (Block 85).
  document.getElementById("schema-upload-cancel").addEventListener("click", () => {
    document.getElementById("schema-upload-dialog").close();
  });
  document.getElementById("schema-upload-form").addEventListener("submit", uploadSchema);

  await refreshTopics();
}

async function refreshTopics() {
  const v = document.getElementById("vendor-select").value;
  const tbody = document.querySelector("#topics-table tbody");
  const empty = document.getElementById("topics-empty");
  const table = document.getElementById("topics-table");
  tbody.innerHTML = '<tr><td colspan="7" role="status">Loading topics…</td></tr>';
  if (empty) empty.classList.add("hidden");
  if (table) table.classList.remove("hidden");
  try {
    const data = await api(`/api/orgs/${encodeURIComponent(v)}/topics`);
    if (!data.topics || data.topics.length === 0) {
      // Name the slot. Most slots on a populated goatnet have no topics
      // registered, so stepping through the picker showed a screen that never
      // changed a pixel — indistinguishable from a selector that isn't wired
      // up, which is how this got reported as "selecting a slot does nothing".
      if (empty) {
        empty.textContent = `No topics registered under ${v}. The registry answered — this ${LEX.org} has declared none. Register one below, or upload a YAML file in bulk.`;
        empty.classList.remove("hidden");
      }
      if (table) table.classList.add("hidden");
      return;
    }
    if (empty) empty.classList.add("hidden");
    if (table) table.classList.remove("hidden");
    tbody.innerHTML = "";
    for (const t of data.topics) {
      const tr = document.createElement("tr");
      const lastSeen = t.last_seen_at ? new Date(t.last_seen_at * 1000).toISOString().slice(0, 19) + "Z" : "—";
      tr.innerHTML = `
        <td><a href="/inspector.html?key=${encodeURIComponent(t.key_pattern)}"><code>${escapeHTML(t.key_pattern)}</code></a></td>
        <td>${escapeHTML(t.format)}</td>
        <td>${renderSchemaCell(t)}</td>
        <td>${t.publish_to_storage ? "yes" : "no"}</td>
        <td>${lastSeen}</td>
        <td>${t.publish_count}</td>
        <td>
          <button data-id="${t.id}" data-key="${escapeHTML(t.key_pattern)}" class="schema-upload-btn" title="Upload a schema descriptor for this topic">upload schema</button>
          <button data-id="${t.id}" data-key="${escapeHTML(t.key_pattern)}" class="retire-btn" title="Soft-delete: tombstone the row for audit. The key_pattern stays reserved; re-registering the same name is refused.">retire</button>
          <button data-id="${t.id}" data-key="${escapeHTML(t.key_pattern)}" class="delete-btn" title="Hard-delete: row is gone, schema files removed, the key_pattern is free for reuse. Permanent. No undo.">delete</button>
        </td>
      `;
      tbody.appendChild(tr);
    }
    for (const btn of document.querySelectorAll(".retire-btn")) {
      btn.addEventListener("click", async (e) => {
        const id = e.target.getAttribute("data-id");
        const key = e.target.getAttribute("data-key");
        if (!confirm(`Retire topic "${key}"?\n\nThe row is tombstoned but stays in the registry for audit. The key_pattern stays reserved — re-registering "${key}" will be refused. Use Delete instead if you want to free the name for reuse.`)) return;
        try {
          await api(`/api/orgs/${encodeURIComponent(v)}/topics/${id}`, { method: "DELETE" });
          await refreshTopics();
        } catch (err) {
          alert("retire failed: " + err.message);
        }
      });
    }
    for (const btn of document.querySelectorAll(".delete-btn")) {
      btn.addEventListener("click", async (e) => {
        const id = e.target.getAttribute("data-id");
        const key = e.target.getAttribute("data-key");
        if (!confirm(`DELETE topic "${key}"?\n\nThe row will be permanently removed, every uploaded schema descriptor for it will be deleted from disk, and the key_pattern will be free for re-registration. This cannot be undone — audit trail survives only in audit_events. Use Retire instead if you want to keep the row as a tombstone.`)) return;
        try {
          await api(`/api/orgs/${encodeURIComponent(v)}/topics/${id}?purge=true`, { method: "DELETE" });
          await refreshTopics();
        } catch (err) {
          alert("delete failed: " + err.message);
        }
      });
    }
    for (const btn of document.querySelectorAll(".schema-upload-btn")) {
      btn.addEventListener("click", (e) => {
        const id = e.target.getAttribute("data-id");
        const key = e.target.getAttribute("data-key");
        openSchemaUploadDialog(id, key);
      });
    }
    for (const a of document.querySelectorAll(".schema-download-link")) {
      a.addEventListener("click", (e) => {
        e.preventDefault();
        downloadSchema(a.getAttribute("data-key"), a.getAttribute("data-version"));
      });
    }
  } catch (e) {
    showRequestError(e);
    // The error row goes into #topics-table, so un-hide it and drop the empty
    // note first: a failure straight after an empty slot used to write the row
    // into a table still carrying .hidden, leaving "no topics" on screen and
    // the real reason (an expired token, say) invisible.
    if (empty) empty.classList.add("hidden");
    if (table) table.classList.remove("hidden");
    tbody.innerHTML = `<tr><td colspan="7" class="cell-error">Error: ${escapeHTML(e.message)}</td></tr>`;
  }
}

async function createTopic(e) {
  e.preventDefault();
  const v = document.getElementById("vendor-select").value;
  const out = document.getElementById("topic-create-result");
  out.classList.remove("hidden");

  // Build the format string. For protobuf, splice in the message name from
  // the dedicated field and refuse the literal "MyMessage" placeholder
  // client-side — the server-side format_mismatch validator is the real
  // gate, but it isn't deployed everywhere yet, and a bad message name
  // here lands the topic in the F-162 state where panoscope/the viewer
  // can't decode anything.
  let format = document.getElementById("t-format").value;
  if (format.startsWith("protobuf:")) {
    const msg = document.getElementById("t-format-msgname").value.trim();
    if (!msg || msg === "MyMessage") {
      out.classList.add("error");
      out.textContent =
        "Enter the protobuf message name (a top-level message in the .fds you'll upload) — not the 'MyMessage' placeholder. See the yellow callout above.";
      return;
    }
    format = "protobuf:" + msg;
  } else if (format === "xml:") {
    // Schema-bound XML: splice in the root element name. The server-side
    // format_mismatch validator against the uploaded .xsd is the real
    // gate; this just refuses an empty root element client-side.
    const el = document.getElementById("t-format-elname").value.trim();
    if (!el) {
      out.classList.add("error");
      out.textContent =
        "Enter the XML root element name (a top-level <xs:element> in the .xsd you'll upload). For opaque XML with no schema, pick the plain 'xml' format instead.";
      return;
    }
    format = "xml:" + el;
  }
  out.classList.remove("error");
  out.textContent = "Registering...";
  try {
    const resp = await api(`/api/orgs/${encodeURIComponent(v)}/topics`, {
      method: "POST",
      body: JSON.stringify({
        key_pattern: document.getElementById("t-key").value.trim(),
        format: format,
        schema_ref: document.getElementById("t-schema-ref").value.trim(),
        description: document.getElementById("t-desc").value.trim(),
        publish_to_storage: document.getElementById("t-storage").checked,
      }),
    });
    out.textContent = JSON.stringify(resp, null, 2);
    await refreshTopics();
  } catch (err) {
    out.textContent = "Error: " + err.message;
  }
}

// --- schema upload / fetch (Block 85) --------------------------------

// renderSchemaCell renders the per-topic "Schema" column. Three states:
//
//   - schema_ref starts with "attest/schema/"  → a body has been uploaded;
//     show the canonical pointer + a download link to the latest version
//     (the version is the trailing /v<N> segment).
//
//   - schema_ref non-empty but not in the canonical attest/schema/ shape
//     → vendor-supplied free-text pointer; show it verbatim, no download.
//
//   - empty schema_ref → render an em dash.
function renderSchemaCell(t) {
  if (!t.schema_ref) return "<em>—</em>";
  const ref = t.schema_ref;
  if (!ref.startsWith("attest/schema/")) {
    return "<code>" + escapeHTML(ref) + "</code>";
  }
  // Parse trailing /v<N> as the schema version; everything between
  // "attest/schema/" and that suffix is the topic key.
  const stripped = ref.slice("attest/schema/".length);
  const m = stripped.match(/^(.+)\/(v\d+)$/);
  if (!m) return "<code>" + escapeHTML(ref) + "</code>";
  const key = m[1];
  const version = m[2];
  return `<code>${escapeHTML(ref)}</code> ` +
    `<a href="/schema.html?key=${encodeURIComponent(key)}&version=${encodeURIComponent(version)}" class="schema-view-link" title="View the ${escapeHTML(version)} schema as a rendered data dictionary">view</a> ` +
    `<a href="#" class="schema-download-link" data-key="${escapeHTML(key)}" data-version="${escapeHTML(version)}" title="Download the ${escapeHTML(version)} schema body">↓</a>`;
}

function openSchemaUploadDialog(topicID, keyPattern) {
  const dlg = document.getElementById("schema-upload-dialog");
  document.getElementById("schema-upload-key").textContent = keyPattern;
  dlg.dataset.topicId = topicID;
  dlg.dataset.keyPattern = keyPattern;
  document.getElementById("schema-version").value = "";
  document.getElementById("schema-compatibility").value = "none";
  document.getElementById("schema-file").value = "";
  const out = document.getElementById("schema-upload-result");
  out.classList.add("hidden");
  out.textContent = "";
  dlg.showModal();
}

async function uploadSchema(e) {
  e.preventDefault();
  const dlg = document.getElementById("schema-upload-dialog");
  const topicID = dlg.dataset.topicId;
  const vendor = document.getElementById("vendor-select").value;
  const version = document.getElementById("schema-version").value.trim();
  const compatibility = document.getElementById("schema-compatibility").value;
  const fileInput = document.getElementById("schema-file");
  const out = document.getElementById("schema-upload-result");
  out.classList.remove("hidden");

  if (!fileInput.files || fileInput.files.length === 0) {
    out.textContent = "Pick a .proto, .fds, or .xsd file first.";
    return;
  }
  if (!/^v\d+$/.test(version)) {
    out.textContent = "Version must match v<N> (e.g. v1, v2).";
    return;
  }

  // .proto → server compiles in-process via bufbuild/protocompile.
  // .fds → persisted verbatim as a FileDescriptorSet.
  // .xsd → persisted verbatim as an XML Schema Definition (the schema
  // half of an xml:<RootElement> topic).
  // The format= field tells the server which kind it is.
  const fileName = fileInput.files[0].name.toLowerCase();
  let format = "protobuf-fds";
  if (fileName.endsWith(".proto")) format = "protobuf-source";
  else if (fileName.endsWith(".xsd")) format = "xsd";

  const fd = new FormData();
  fd.append("file", fileInput.files[0]);
  fd.append("version", version);
  fd.append("compatibility", compatibility);
  fd.append("format", format);

  const key = hasAuth();
  out.textContent = "Uploading…";
  try {
    const res = await request(
      `/api/orgs/${encodeURIComponent(vendor)}/topics/${encodeURIComponent(topicID)}/schemas`,
      {
        method: "POST",
        headers: {},
        body: fd,
      });
    const body = await readResponse(res);
    if (!res.ok) {
      out.textContent = "Error: " + (body?.error?.message || `HTTP ${res.status}`);
      return;
    }
    out.textContent = JSON.stringify(body, null, 2);
    // Refresh the table so the new schema_ref + download link render.
    await refreshTopics();
    // Auto-close after a short pause so the operator sees the success.
    setTimeout(() => dlg.close(), 800);
  } catch (err) {
    out.textContent = "Error: " + err.message;
  }
}

// downloadSchema fetches /api/schemas/<key>?version=<v> with bearer auth
// and saves the response body as a .fds file. Plain <a download> can't
// add Authorization headers; the blob dance mirrors exportTopics.
function downloadSchema(keyPattern, version) {
  const k = hasAuth();
  if (!k) { alert("Sign in first."); return; }
  const url = `/api/schemas/${keyPattern}?version=${encodeURIComponent(version)}`;
  request(url, { headers: {} })
    .then(async (r) => {
      if (!r.ok) {
        await readResponse(r);
      }
      const blob = await r.blob();
      const a = document.createElement("a");
      a.href = URL.createObjectURL(blob);
      // Filename mirrors the on-disk layout: <last-key-segment>-<version>.fds
      const lastSegment = keyPattern.split("/").filter(Boolean).pop() || "schema";
      a.download = `${lastSegment}-${version}.fds`;
      document.body.appendChild(a);
      a.click();
      setTimeout(() => { URL.revokeObjectURL(a.href); a.remove(); }, 0);
    })
    .catch((err) => alert("download failed: " + err.message));
}

// --- bulk export -----------------------------------------------------

function exportTopics(format) {
  const v = document.getElementById("vendor-select").value;
  const key = hasAuth();
  if (!v || !key) { alert(`Pick ${/^[aeiou]/i.test(LEX.org) ? "an" : "a"} ${LEX.org} and sign in first.`); return; }
  // We can't add Authorization headers to a plain <a download>, so fetch
  // the blob, then save it.
  const url = `/api/orgs/${encodeURIComponent(v)}/topics/export?format=${encodeURIComponent(format)}`;
  request(url)
    .then(async (r) => {
      if (!r.ok) {
        await readResponse(r);
      }
      const blob = await r.blob();
      const filename = (r.headers.get("Content-Disposition") || "").match(/filename="([^"]+)"/)?.[1]
        || `${v}-topics.${format}`;
      const a = document.createElement("a");
      a.href = URL.createObjectURL(blob);
      a.download = filename;
      document.body.appendChild(a);
      a.click();
      setTimeout(() => { URL.revokeObjectURL(a.href); a.remove(); }, 0);
    })
    .catch((e) => alert("export failed: " + e.message));
}

// --- bulk import -----------------------------------------------------

async function bulkImport(dryRun) {
  const v = document.getElementById("vendor-select").value;
  const fileInput = document.getElementById("bulk-file");
  const mode = document.getElementById("bulk-mode").value;
  const out = document.getElementById("bulk-result");
  out.classList.remove("hidden");

  if (!fileInput.files || fileInput.files.length === 0) {
    out.textContent = "Pick a YAML or JSON file first.";
    return;
  }

  const f = fileInput.files[0];
  const text = await f.text();
  const ct = guessContentType(f.name);
  const key = hasAuth();
  const params = new URLSearchParams({ mode });
  if (dryRun) params.set("dry_run", "true");

  out.textContent = (dryRun ? "Previewing…" : "Applying…");
  try {
    const res = await request(
      `/api/orgs/${encodeURIComponent(v)}/topics/import?${params.toString()}`,
      {
        method: "POST",
        headers: {
          "Content-Type": ct,
        },
        body: text,
      });
    const data = await readResponse(res);
    if (!res.ok) {
      out.textContent = "Error: " + (data?.error?.message || `HTTP ${res.status}`);
      return;
    }
    out.textContent = renderDiff(data);
    if (!dryRun) await refreshTopics();
  } catch (e) {
    out.textContent = "Error: " + e.message;
  }
}

function guessContentType(filename) {
  const n = filename.toLowerCase();
  if (n.endsWith(".yaml") || n.endsWith(".yml")) return "application/yaml";
  if (n.endsWith(".json")) return "application/json";
  return "application/yaml";
}

function renderDiff(diff) {
  const lines = [];
  lines.push(`vendor: ${diff.vendor}`);
  lines.push(`mode:   ${diff.mode}`);
  lines.push(`dry_run: ${diff.dry_run}`);
  lines.push(`summary: ${JSON.stringify(diff.summary)}`);
  lines.push("");
  lines.push("entries:");
  for (const e of (diff.entries || [])) {
    lines.push(`  ${e.action.padEnd(10)} ${e.key_pattern}`);
  }
  return lines.join("\n");
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", initTopics);
} else {
  initTopics();
}
