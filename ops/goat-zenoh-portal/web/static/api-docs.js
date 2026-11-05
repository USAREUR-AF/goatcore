// SPDX-License-Identifier: Apache-2.0
// Swagger owns schema rendering, deep links and request execution. This shell
// derives navigation and the reading overview from its loaded specification.
window.addEventListener("DOMContentLoaded", () => {
  const search = document.getElementById("api-search");
  const status = document.getElementById("api-status");
  const browse = document.getElementById("api-browse");
  const narrow = window.matchMedia("(max-width: 720px)");
  const syncBrowse = () => { browse.open = !narrow.matches; };
  syncBrowse();
  narrow.addEventListener("change", syncBrowse);
  function showLoadError() {
    search.disabled = true;
    status.textContent = "Unable to load the API reference. See the specification errors below.";
    status.hidden = false;
  }
  // Failed downloads never call onComplete. Swagger's loading action covers
  // both HTTP errors and rejected fetches while keeping its own diagnostics.
  const loadStatusPlugin = () => ({
    statePlugins: {
      spec: {
        wrapActions: {
          updateLoadingStatus: (originalAction) => (loadingStatus) => {
            const result = originalAction(loadingStatus);
            if (loadingStatus === "failed") showLoadError();
            return result;
          }
        }
      }
    }
  });
  function filter() {
    if (search.disabled) return;
    const query = search.value.trim().toLowerCase();
    let count = 0;
    document.querySelectorAll("#swagger-ui .opblock-tag-section").forEach((group) => {
      // Collapsed tags unmount their operations. Open them before searching,
      // then let the render observer apply the query to the mounted rows.
      const collapsed = group.querySelector('.opblock-tag button[aria-expanded="false"]');
      if (query && collapsed) collapsed.click();
      let matches = 0;
      group.querySelectorAll(".opblock").forEach((operation) => {
        const summary = operation.querySelector(".opblock-summary");
        operation.hidden = !summary.textContent.toLowerCase().includes(query);
        if (!operation.hidden) matches++;
      });
      group.hidden = Boolean(query) && matches === 0;
      count += matches;
    });
    status.textContent = query ? `${count} matching endpoint${count === 1 ? "" : "s"}` : "";
    status.hidden = !query;
  }
  search.addEventListener("input", filter);
  window.ui = SwaggerUIBundle({
    url: "/openapi.yaml",
    dom_id: "#swagger-ui",
    deepLinking: true,
    presets: [SwaggerUIBundle.presets.apis, SwaggerUIStandalonePreset],
    plugins: [SwaggerUIBundle.plugins.DownloadUrl, loadStatusPlugin],
    layout: "BaseLayout",
    defaultModelsExpandDepth: 0,
    docExpansion: "list",
    tryItOutEnabled: true,
    persistAuthorization: false,
    onComplete: () => {
      const spec = window.ui.specSelectors.specJson().toJS();
      if (!spec.info || !spec.paths) {
        showLoadError();
        return;
      }
      document.getElementById("api-title").textContent = spec.info.title;
      document.getElementById("api-document-title").textContent = spec.info.title;
      const version = `Version ${spec.info.version} · OpenAPI ${spec.openapi || spec.swagger}`;
      document.getElementById("api-version").textContent = version;
      const metadata = document.getElementById("api-metadata");
      metadata.textContent = version;
      const license = document.querySelector("#swagger-ui .info__license");
      if (license) metadata.append(license.cloneNode(true));
      const description = document.querySelector("#swagger-ui .info .description .renderedMarkdown");
      if (description) {
        const paragraphs = Array.from(description.children, (child) => child.cloneNode(true));
        document.getElementById("api-introduction").replaceChildren(...paragraphs.slice(0, 1));
        document.getElementById("api-conventions-body").replaceChildren(...paragraphs.slice(1));
      }
      const groups = document.getElementById("api-groups");
      groups.replaceChildren();
      document.querySelectorAll("#swagger-ui .opblock-tag-section").forEach((group, index) => {
        const source = group.querySelector(".opblock-tag a");
        if (!source) return;
        if (index === 0) group.id = "api-endpoints";
        const link = document.createElement("a");
        link.href = source.getAttribute("href");
        const name = document.createElement("span");
        name.textContent = source.textContent;
        const count = document.createElement("small");
        count.textContent = group.querySelectorAll(".opblock").length;
        link.append(name, count);
        link.addEventListener("click", () => { search.value = ""; filter(); });
        groups.append(link);
      });
      const servers = document.querySelector("#swagger-ui .scheme-container");
      if (servers) servers.id = "api-servers";
      const models = document.querySelector("#swagger-ui .models");
      if (models) models.id = "api-models";
      new MutationObserver(filter).observe(document.getElementById("swagger-ui"), {
        childList: true, subtree: true
      });
      search.disabled = false;
      filter();
    }
  });
});
